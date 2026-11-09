use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Deserialize;

use crate::Error;

/// Environment variables with this prefix override the file, so
/// `HIPHILE_SERVER` replaces `server`.
const ENV_PREFIX: &str = "HIPHILE";

/// Held while `config.toml` is read, changed and written back, so two saves
/// in this process never interleave and lose one's change.
static EDITING: Mutex<()> = Mutex::new(());
/// Numbers temporary files, so no two writes ever share one.
static TEMPORARY: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    server: String,
    username: String,
}

impl Profile {
    /// The same for each way of writing the server's address that differs only in the case of
    /// its scheme and host, or in slashes at its end.
    pub fn new(server: impl Into<String>, username: impl Into<String>) -> Self {
        Self {
            server: server_address(&server.into()),
            username: username.into(),
        }
    }

    pub fn server(&self) -> &str {
        &self.server
    }

    pub fn username(&self) -> &str {
        &self.username
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance {
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LyricsHighlight {
    #[default]
    ByWord,
    ByLine,
}

pub const SAMPLE_RATE_MULTIPLES: [u32; 4] = [2, 4, 8, 16];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SampleRate {
    #[default]
    MatchSong,
    /// In Hz.
    Fixed(u32),
    /// One of [`SAMPLE_RATE_MULTIPLES`] of each song's own rate.
    SongMultiple(u32),
}

impl SampleRate {
    /// The form `sample_rate` takes in the file: Hz, or `x` and a multiple. `0`
    /// means each song's own rate.
    pub fn parse(text: &str) -> Option<Self> {
        if let Some(multiple) = text.strip_prefix('x') {
            return multiple
                .parse()
                .ok()
                .filter(|multiple| SAMPLE_RATE_MULTIPLES.contains(multiple))
                .map(Self::SongMultiple);
        }
        match text.parse().ok()? {
            0 => Some(Self::MatchSong),
            rate => Some(Self::Fixed(rate)),
        }
    }

    /// What [`Self::parse`] reads back as this.
    pub fn text(self) -> String {
        match self {
            Self::MatchSong => "0".to_owned(),
            Self::Fixed(rate) => rate.to_string(),
            Self::SongMultiple(multiple) => format!("x{multiple}"),
        }
    }
}

pub const RESAMPLING_FFT_SIZES: [u32; 7] = [256, 512, 1024, 2048, 4096, 8192, 16384];
pub const R8BRAIN_TRANSITION_BANDS: std::ops::RangeInclusive<f64> = 0.5..=45.0;
pub const R8BRAIN_ATTENUATIONS: std::ops::RangeInclusive<f64> = 49.0..=218.0;
pub const R8BRAIN_MAX_INPUT_FRAMES: std::ops::RangeInclusive<u32> = 1..=1_048_576;
pub const R8BRAIN_FILTER_FRACTIONS: std::ops::RangeInclusive<u32> = 5..=65_536;
pub const R8BRAIN_CACHE_SIZES: std::ops::RangeInclusive<u32> = 2..=4096;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum R8brainPhase {
    #[default]
    Linear,
    Minimum,
}

impl R8brainPhase {
    fn name(self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::Minimum => "minimum",
        }
    }
}

/// What decides the sound of the `r8brain` resampling.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct R8brainFilter {
    /// In percent
    pub transition_band: f64,
    /// In decibels
    pub attenuation: f64,
    pub phase: R8brainPhase,
    pub whole_stepping: bool,
    /// `None` lets the resampler choose.
    pub filter_fractions: Option<u32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum R8brainQuality {
    Low,
    Medium,
    #[default]
    High,
    Max,
    Custom(R8brainFilter),
}

impl R8brainQuality {
    pub fn filter(self) -> R8brainFilter {
        let preset = |transition_band, attenuation| R8brainFilter {
            transition_band,
            attenuation,
            phase: R8brainPhase::Linear,
            whole_stepping: true,
            filter_fractions: None,
        };
        match self {
            // The delay follows the FFT block, whose size goes by powers of two. These each reach a
            // longer block than the last: less of a change can take more delay and buy nothing.
            Self::Low => preset(5.0, 136.45),
            // The narrowest tenth of a percent that keeps this block from any common song rate to
            // any device rate, as 2.3 reaches the next one from 48 to 44.1 kHz
            Self::Medium => preset(2.4, 136.45),
            // What the original has for 24-bit and 32-bit floating point
            Self::High => preset(2.0, 180.15),
            // The ends of the original's ranges
            Self::Max => preset(0.5, 218.0),
            Self::Custom(filter) => filter,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Max => "max",
            Self::Custom(_) => "custom",
        }
    }
}

/// What the file says about the `r8brain` resampling, kept while another one is
/// in use. Only the quality changes the sound.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct R8brainParameters {
    pub quality: R8brainQuality,
    pub performance: R8brainPerformance,
}

/// Of the `r8brain` resampling, what leaves the sound alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct R8brainPerformance {
    pub max_input_frames: u32,
    pub extended_fft: bool,
    pub filter_cache_size: u32,
    pub filter_bank_cache_size: u32,
}

impl Default for R8brainPerformance {
    fn default() -> Self {
        Self {
            max_input_frames: 1024,
            extended_fft: false,
            filter_cache_size: 96,
            filter_bank_cache_size: 12,
        }
    }
}

/// In bits
pub const SOXR_PRECISIONS: std::ops::RangeInclusive<f64> = 15.0..=33.0;
/// As fractions of the Nyquist frequency. With the stop-band from the Nyquist frequency on, these
/// are the ends of the widths that the transition band may have.
pub const SOXR_PASSBAND_ENDS: std::ops::RangeInclusive<f64> = 0.5..=0.998;
pub const SOXR_COEF_TABLE_KBS: std::ops::RangeInclusive<u32> = 1..=65_536;
pub const SOXR_MIN_DFT_SIZES: [u32; 8] = [256, 512, 1024, 2048, 4096, 8192, 16_384, 32_768];
pub const SOXR_LARGE_DFT_SIZES: [u32; 13] = [
    256, 512, 1024, 2048, 4096, 8192, 16_384, 32_768, 65_536, 131_072, 262_144, 524_288, 1_048_576,
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SoxrPhase {
    #[default]
    Linear,
    /// Between minimum and linear
    Intermediate,
    Minimum,
}

impl SoxrPhase {
    fn name(self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::Intermediate => "intermediate",
            Self::Minimum => "minimum",
        }
    }
}

/// What decides the sound of the `soxr` resampling.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct SoxrFilter {
    /// In bits
    pub precision: f64,
    /// As a fraction of the lower rate's Nyquist frequency
    pub passband_end: f64,
    pub phase: SoxrPhase,
    /// Computes in 64-bit floating point at 20 bits or less too, as it always does above.
    pub double_precision: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum SoxrQuality {
    Low,
    Medium,
    High,
    #[default]
    VeryHigh,
    Custom(SoxrFilter),
}

impl SoxrQuality {
    /// The presets are libsoxr's LQ, MQ, HQ and VHQ, to the bit.
    pub fn filter(self) -> SoxrFilter {
        let preset = |precision, passband_end| SoxrFilter {
            precision,
            passband_end,
            phase: SoxrPhase::Linear,
            double_precision: false,
        };
        match self {
            Self::Low => preset(16.0, 0.67626953125),
            Self::Medium => preset(16.0, 0.9158810491765671),
            Self::High => preset(20.0, 0.9137436532733066),
            Self::VeryHigh => preset(28.0, 0.9113283198951663),
            Self::Custom(filter) => filter,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::VeryHigh => "very-high",
            Self::Custom(_) => "custom",
        }
    }
}

/// How the `soxr` resampling finds the phases of its filter that it does not keep.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SoxrInterpolation {
    /// Keeps every phase that a ratio of small whole numbers takes, if they fit in the table.
    #[default]
    Auto,
    Linear,
    Cubic,
}

impl SoxrInterpolation {
    fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Linear => "linear",
            Self::Cubic => "cubic",
        }
    }
}

/// What the file says about the `soxr` resampling, kept while another one is
/// in use. Only the quality changes the sound.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct SoxrParameters {
    pub quality: SoxrQuality,
    pub performance: SoxrPerformance,
}

/// Of the `soxr` resampling, what changes the sound by no more than rounding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SoxrPerformance {
    /// In kB
    pub coef_table_kb: u32,
    pub coef_interpolation: SoxrInterpolation,
    /// One of [`SOXR_MIN_DFT_SIZES`]
    pub min_dft_size: u32,
    /// One of [`SOXR_LARGE_DFT_SIZES`]
    pub large_dft_size: u32,
}

impl Default for SoxrPerformance {
    fn default() -> Self {
        Self {
            coef_table_kb: 400,
            coef_interpolation: SoxrInterpolation::Auto,
            min_dft_size: 1024,
            large_dft_size: 131_072,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum ResamplingQuality {
    #[default]
    Medium,
    High,
    Max,
    Custom {
        fft_size: u32,
        window: ResamplingWindow,
    },
    /// Takes no FFT size or window of its own, and what it answers for those is
    /// what a custom quality starts from after it.
    R8brain(R8brainParameters),
    /// As [`Self::R8brain`]
    Soxr(SoxrParameters),
}

impl ResamplingQuality {
    pub fn fft_size(self) -> u32 {
        match self {
            Self::Medium | Self::R8brain(_) | Self::Soxr(_) => 256,
            Self::High => 1024,
            Self::Max => 4096,
            Self::Custom { fft_size, .. } => fft_size,
        }
    }

    pub fn window(self) -> ResamplingWindow {
        match self {
            Self::Medium | Self::High | Self::Max | Self::R8brain(_) | Self::Soxr(_) => {
                ResamplingWindow::BlackmanHarris2
            }
            Self::Custom { window, .. } => window,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Medium => "medium",
            Self::High => "high",
            Self::Max => "max",
            Self::Custom { .. } => "custom",
            Self::R8brain(_) => "r8brain",
            Self::Soxr(_) => "soxr",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResamplingWindow {
    Hann,
    Hann2,
    Blackman,
    Blackman2,
    BlackmanHarris,
    BlackmanHarris2,
}

impl ResamplingWindow {
    pub const ALL: [Self; 6] = [
        Self::Hann,
        Self::Hann2,
        Self::Blackman,
        Self::Blackman2,
        Self::BlackmanHarris,
        Self::BlackmanHarris2,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Hann => "hann",
            Self::Hann2 => "hann-2",
            Self::Blackman => "blackman",
            Self::Blackman2 => "blackman-2",
            Self::BlackmanHarris => "blackman-harris",
            Self::BlackmanHarris2 => "blackman-harris-2",
        }
    }
}

pub const DSD_SINC_LENGTHS: [u32; 4] = [256, 512, 1024, 2048];
pub const DSD_ORDERS: [u8; 4] = [3, 5, 7, 9];
const DSD_NOISE_GAINS: std::ops::RangeInclusive<f64> = 1.2..=1.5;
const DSD_CUTOFFS: std::ops::RangeInclusive<f64> = 0.5..=1.0;
const DSD_DITHER_DBS: std::ops::RangeInclusive<f64> = -160.0..=-20.0;
const DSD_INPUT_LEVEL_DBS: std::ops::RangeInclusive<f64> = -24.0..=6.0;
const DSD_CEILING_DBS: std::ops::RangeInclusive<f64> = -6.0..=3.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DsdRate {
    Dsd64,
    Dsd128,
    Dsd256,
}

impl DsdRate {
    pub const ALL: [Self; 3] = [Self::Dsd64, Self::Dsd128, Self::Dsd256];

    /// As a multiple of 44.1 kHz, which is how DSD rates are named.
    pub fn multiple(self) -> u16 {
        match self {
            Self::Dsd64 => 64,
            Self::Dsd128 => 128,
            Self::Dsd256 => 256,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DsdFilter {
    Sinc {
        length: u32,
        window: ResamplingWindow,
        /// As a fraction of the source's Nyquist frequency. `None` lets the
        /// resampler choose.
        cutoff: Option<f64>,
    },
    Fft {
        size: u32,
        window: ResamplingWindow,
    },
}

impl DsdFilter {
    pub fn window(self) -> ResamplingWindow {
        match self {
            Self::Sinc { window, .. } | Self::Fft { window, .. } => window,
        }
    }

    /// The same filter with `window`.
    pub fn with_window(self, window: ResamplingWindow) -> Self {
        match self {
            Self::Sinc { length, cutoff, .. } => Self::Sinc {
                length,
                window,
                cutoff,
            },
            Self::Fft { size, .. } => Self::Fft { size, window },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct DsdParameters {
    pub filter: DsdFilter,
    pub order: u8,
    pub noise_gain: f64,
    pub wide_noise_band: bool,
    pub optimized_zeros: bool,
    /// `None` for no dither.
    pub dither_db: Option<f64>,
    pub input_level_db: f64,
    /// `None` for no limiter.
    pub limiter_ceiling_db: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum DsdQuality {
    Low,
    Medium,
    #[default]
    High,
    Max,
    Custom(DsdParameters),
}

impl DsdQuality {
    pub fn parameters(self) -> DsdParameters {
        let preset = |order, length| DsdParameters {
            filter: DsdFilter::Sinc {
                length,
                window: ResamplingWindow::BlackmanHarris2,
                cutoff: None,
            },
            order,
            noise_gain: 1.4,
            wide_noise_band: false,
            optimized_zeros: true,
            dither_db: None,
            input_level_db: 0.0,
            limiter_ceiling_db: Some(1.0),
        };
        match self {
            Self::Low => preset(5, 256),
            Self::Medium => preset(7, 512),
            Self::High => preset(7, 1024),
            // Not order 9: at these noise gains it measures no quieter than 7
            Self::Max => preset(7, 2048),
            Self::Custom(parameters) => parameters,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Max => "max",
            Self::Custom(_) => "custom",
        }
    }
}

/// What the file says about DSD output. It is on for the one device it was
/// turned on for, since DoP on a device that cannot decode it is loud noise.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct DsdOutput {
    pub is_on: bool,
    pub device: Option<String>,
    /// `None` until one is chosen, which means DSD256.
    pub rate: Option<DsdRate>,
    /// Plays songs of the 48 kHz family at the 48 kHz family's DSD rates.
    pub matches_song_family: bool,
    pub quality: DsdQuality,
    /// Raises the level from silence where sound would otherwise start at
    /// full level.
    pub fades_in: bool,
}

impl Default for DsdOutput {
    fn default() -> Self {
        Self {
            is_on: false,
            device: None,
            rate: None,
            matches_song_family: false,
            quality: DsdQuality::default(),
            fades_in: true,
        }
    }
}

pub const LONGEST_CROSSFADE_SECONDS: u32 = 12;

pub const CROSSFEED_CUTOFFS_HZ: std::ops::RangeInclusive<u32> = 300..=2000;
/// In dB
pub const CROSSFEED_SEPARATIONS_DB: std::ops::RangeInclusive<f64> = 1.0..=15.0;
/// In dB. A negative value lowers the right channel and a positive one the left.
pub const BALANCE_DBS: std::ops::RangeInclusive<f64> = -12.0..=12.0;

/// What decides the sound of bs2b's crossfeed.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct CrossfeedFilter {
    pub cutoff_hz: u32,
    /// How much lower the bass fed to the other channel is than the channel's own, in dB
    pub separation_db: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum CrossfeedPreset {
    /// bs2b's default
    #[default]
    Bs2b,
    ChuMoy,
    JanMeier,
    Custom(CrossfeedFilter),
}

impl CrossfeedPreset {
    pub fn filter(self) -> CrossfeedFilter {
        let preset = |cutoff_hz, separation_db| CrossfeedFilter {
            cutoff_hz,
            separation_db,
        };
        match self {
            Self::Bs2b => preset(700, 4.5),
            Self::ChuMoy => preset(700, 6.0),
            Self::JanMeier => preset(650, 9.5),
            Self::Custom(filter) => filter,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Bs2b => "bs2b",
            Self::ChuMoy => "chu-moy",
            Self::JanMeier => "jan-meier",
            Self::Custom(_) => "custom",
        }
    }
}

/// What the file says about crossfeed. The preset stays while it is off, to come back to.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct Crossfeed {
    pub is_on: bool,
    pub preset: CrossfeedPreset,
}

/// The keys of `config.toml` this crate reads. Others are left alone.
#[derive(Deserialize)]
struct Settings {
    server: Option<String>,
    username: Option<String>,
    password: Option<String>,
    volume: Option<f32>,
    song_cache_gib: Option<f64>,
    cover_cache_gib: Option<f64>,
    output_device: Option<String>,
    sample_rate: Option<String>,
    exclusive_mode: Option<bool>,
    trim_silence: Option<bool>,
    crossfade_seconds: Option<u32>,
    crossfeed: Option<bool>,
    crossfeed_preset: Option<String>,
    crossfeed_cutoff_hz: Option<u32>,
    crossfeed_separation_db: Option<f64>,
    balance_db: Option<f64>,
    play_pause_fade: Option<bool>,
    skip_instrumentals: Option<bool>,
    resampling: Option<String>,
    resampling_fft_size: Option<u32>,
    resampling_window: Option<String>,
    r8brain_quality: Option<String>,
    r8brain_transition_band: Option<f64>,
    r8brain_attenuation: Option<f64>,
    r8brain_phase: Option<String>,
    r8brain_max_input_frames: Option<u32>,
    r8brain_extended_fft: Option<bool>,
    r8brain_whole_stepping: Option<bool>,
    r8brain_filter_fractions: Option<u32>,
    r8brain_filter_cache_size: Option<u32>,
    r8brain_filter_bank_cache_size: Option<u32>,
    soxr_quality: Option<String>,
    soxr_precision: Option<f64>,
    soxr_passband_end: Option<f64>,
    soxr_phase: Option<String>,
    soxr_double_precision: Option<bool>,
    soxr_coef_table_kb: Option<u32>,
    soxr_coef_interpolation: Option<String>,
    soxr_min_dft_size: Option<u32>,
    soxr_large_dft_size: Option<u32>,
    output_format: Option<String>,
    dop_device: Option<String>,
    dsd_rate: Option<u16>,
    dsd_base_rate: Option<String>,
    dsd_fade_in: Option<bool>,
    dsd_quality: Option<String>,
    dsd_filter: Option<String>,
    dsd_sinc_length: Option<u32>,
    dsd_fft_size: Option<u32>,
    dsd_window: Option<String>,
    dsd_cutoff: Option<f64>,
    dsd_order: Option<u8>,
    dsd_noise_gain: Option<f64>,
    dsd_noise_band: Option<u32>,
    dsd_optimized_zeros: Option<bool>,
    dsd_dither_db: Option<f64>,
    dsd_input_level_db: Option<f64>,
    dsd_limiter_ceiling_db: Option<String>,
    appearance: Option<String>,
    lyrics_highlight: Option<String>,
    romaji_search: Option<bool>,
    discord_presence: Option<bool>,
    auto_update: Option<bool>,
    imgbb_api_key: Option<String>,
    profiles: Option<Lenient<Vec<Lenient<SavedProfile>>>>,
}

/// A value read if it can be, so that one written wrong by hand loses only itself.
#[derive(Deserialize)]
#[serde(untagged)]
enum Lenient<T> {
    Read(T),
    Unread(serde::de::IgnoredAny),
}

impl<T> Lenient<T> {
    fn read(self) -> Option<T> {
        match self {
            Self::Read(value) => Some(value),
            Self::Unread(_) => None,
        }
    }
}

#[derive(Deserialize)]
struct SavedProfile {
    server: Option<String>,
    username: Option<String>,
}

const DSD_PARAMETER_KEYS: [&str; 12] = [
    "dsd_filter",
    "dsd_sinc_length",
    "dsd_fft_size",
    "dsd_window",
    "dsd_cutoff",
    "dsd_order",
    "dsd_noise_gain",
    "dsd_noise_band",
    "dsd_optimized_zeros",
    "dsd_dither_db",
    "dsd_input_level_db",
    "dsd_limiter_ceiling_db",
];

/// The keys `parameters` come to. What is at its default has none.
fn dsd_parameter_items(parameters: &DsdParameters) -> Vec<(&'static str, toml_edit::Item)> {
    let mut items = vec![(
        "dsd_window",
        toml_edit::value(parameters.filter.window().name()),
    )];
    match parameters.filter {
        DsdFilter::Sinc { length, cutoff, .. } => {
            items.push(("dsd_filter", toml_edit::value("sinc")));
            items.push(("dsd_sinc_length", toml_edit::value(i64::from(length))));
            if let Some(cutoff) = cutoff {
                items.push(("dsd_cutoff", toml_edit::value(cutoff)));
            }
        }
        DsdFilter::Fft { size, .. } => {
            items.push(("dsd_filter", toml_edit::value("fft")));
            items.push(("dsd_fft_size", toml_edit::value(i64::from(size))));
        }
    }
    items.push(("dsd_order", toml_edit::value(i64::from(parameters.order))));
    items.push(("dsd_noise_gain", toml_edit::value(parameters.noise_gain)));
    items.push((
        "dsd_noise_band",
        toml_edit::value(if parameters.wide_noise_band {
            48_000
        } else {
            20_000
        }),
    ));
    items.push((
        "dsd_optimized_zeros",
        toml_edit::value(parameters.optimized_zeros),
    ));
    if let Some(dither_db) = parameters.dither_db {
        items.push(("dsd_dither_db", toml_edit::value(dither_db)));
    }
    items.push((
        "dsd_input_level_db",
        toml_edit::value(parameters.input_level_db),
    ));
    items.push((
        "dsd_limiter_ceiling_db",
        match parameters.limiter_ceiling_db {
            Some(ceiling) => toml_edit::value(ceiling),
            None => toml_edit::value("off"),
        },
    ));
    items
}

/// Writes the keys of `after` that differ from those of `before`, and removes
/// those it no longer has. Without `before` that is all of them.
fn save_dsd_parameters(
    document: &mut toml_edit::DocumentMut,
    before: Option<&DsdParameters>,
    after: Option<&DsdParameters>,
) {
    let before = before.map_or_else(Vec::new, dsd_parameter_items);
    let after = after.map_or_else(Vec::new, dsd_parameter_items);
    let find = |items: &[(&'static str, toml_edit::Item)], key: &str| {
        items
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, item)| item.to_string())
    };
    for key in DSD_PARAMETER_KEYS {
        if before.is_empty() || after.is_empty() || find(&before, key) != find(&after, key) {
            match after.iter().find(|(name, _)| *name == key) {
                Some((_, item)) => document[key] = item.clone(),
                None => {
                    document.remove(key);
                }
            }
        }
    }
}

const CROSSFEED_FILTER_KEYS: [&str; 2] = ["crossfeed_cutoff_hz", "crossfeed_separation_db"];

const CROSSFEED_KEYS: ParameterKeys = ParameterKeys {
    quality: "crossfeed_preset",
    filter: &CROSSFEED_FILTER_KEYS,
};

fn write_crossfeed_changes(
    document: &mut toml_edit::DocumentMut,
    before: Crossfeed,
    after: Crossfeed,
) {
    let is_custom = |crossfeed: Crossfeed| matches!(crossfeed.preset, CrossfeedPreset::Custom(_));
    write_changes(
        document,
        &CROSSFEED_KEYS,
        (crossfeed_items(before).into(), is_custom(before)),
        (crossfeed_items(after).into(), is_custom(after)),
        before.preset != after.preset,
    );
}

/// Only what the preset uses. What is at its default has no key.
fn crossfeed_items(crossfeed: Crossfeed) -> [(&'static str, Option<toml_edit::Item>); 4] {
    let preset = crossfeed.preset;
    let bs2b = CrossfeedPreset::Bs2b.filter();
    let custom = match preset {
        CrossfeedPreset::Custom(filter) => Some(filter),
        _ => None,
    };
    [
        ("crossfeed", crossfeed.is_on.then(|| toml_edit::value(true))),
        (
            "crossfeed_preset",
            (preset != CrossfeedPreset::Bs2b).then(|| toml_edit::value(preset.name())),
        ),
        (
            "crossfeed_cutoff_hz",
            custom
                .filter(|filter| filter.cutoff_hz != bs2b.cutoff_hz)
                .map(|filter| toml_edit::value(i64::from(filter.cutoff_hz))),
        ),
        (
            "crossfeed_separation_db",
            custom
                .filter(|filter| filter.separation_db != bs2b.separation_db)
                .map(|filter| toml_edit::value(filter.separation_db)),
        ),
    ]
}

const R8BRAIN_FILTER_KEYS: [&str; 5] = [
    "r8brain_transition_band",
    "r8brain_attenuation",
    "r8brain_phase",
    "r8brain_whole_stepping",
    "r8brain_filter_fractions",
];

/// The keys of a resampler's parameters, in the order of its items
struct ParameterKeys {
    quality: &'static str,
    filter: &'static [&'static str],
}

/// A resampler's items, each a key and its value or `None` for no key, and whether its quality
/// is custom
type ParameterItems = (Vec<(&'static str, Option<toml_edit::Item>)>, bool);

/// Writes what differs from `before`, the values in use until now, as the rest may have come from
/// the environment. Moving to or from a preset writes every filter key, so that a key left over
/// from another quality cannot come back unnoticed. `quality_changes` also for another custom
/// filter, so that the file says it is custom.
fn write_changes(
    document: &mut toml_edit::DocumentMut,
    keys: &ParameterKeys,
    (before, before_is_custom): ParameterItems,
    (after, after_is_custom): ParameterItems,
    quality_changes: bool,
) {
    let text = |item: &Option<toml_edit::Item>| item.as_ref().map(ToString::to_string);
    let preset_changes = quality_changes && !(before_is_custom && after_is_custom);
    for ((key, old), (_, new)) in before.into_iter().zip(after) {
        let rewrite = if key == keys.quality {
            quality_changes
        } else if keys.filter.contains(&key) {
            preset_changes || text(&old) != text(&new)
        } else {
            text(&old) != text(&new)
        };
        if !rewrite {
            continue;
        }
        match new {
            Some(item) => document[key] = item,
            None => {
                document.remove(key);
            }
        }
    }
}

const R8BRAIN_KEYS: ParameterKeys = ParameterKeys {
    quality: "r8brain_quality",
    filter: &R8BRAIN_FILTER_KEYS,
};

fn write_r8brain_changes(
    document: &mut toml_edit::DocumentMut,
    before: R8brainParameters,
    after: R8brainParameters,
) {
    let is_custom =
        |parameters: R8brainParameters| matches!(parameters.quality, R8brainQuality::Custom(_));
    write_changes(
        document,
        &R8BRAIN_KEYS,
        (r8brain_items(before).into(), is_custom(before)),
        (r8brain_items(after).into(), is_custom(after)),
        before.quality != after.quality,
    );
}

/// Only what the quality uses. What is at its default has no key, and so follows the default if
/// that changes.
fn r8brain_items(parameters: R8brainParameters) -> [(&'static str, Option<toml_edit::Item>); 10] {
    let quality = parameters.quality;
    let high = R8brainQuality::High.filter();
    let custom = match quality {
        R8brainQuality::Custom(filter) => Some(filter),
        _ => None,
    };
    let performance = parameters.performance;
    let default = R8brainPerformance::default();
    [
        (
            "r8brain_quality",
            (quality != R8brainQuality::High).then(|| toml_edit::value(quality.name())),
        ),
        (
            "r8brain_transition_band",
            custom
                .filter(|filter| filter.transition_band != high.transition_band)
                .map(|filter| toml_edit::value(filter.transition_band)),
        ),
        (
            "r8brain_attenuation",
            custom
                .filter(|filter| filter.attenuation != high.attenuation)
                .map(|filter| toml_edit::value(filter.attenuation)),
        ),
        (
            "r8brain_phase",
            custom
                .filter(|filter| filter.phase != high.phase)
                .map(|filter| toml_edit::value(filter.phase.name())),
        ),
        (
            "r8brain_whole_stepping",
            custom
                .filter(|filter| filter.whole_stepping != high.whole_stepping)
                .map(|filter| toml_edit::value(filter.whole_stepping)),
        ),
        (
            "r8brain_filter_fractions",
            custom
                .and_then(|filter| filter.filter_fractions)
                .map(|fractions| toml_edit::value(i64::from(fractions))),
        ),
        (
            "r8brain_max_input_frames",
            (performance.max_input_frames != default.max_input_frames)
                .then(|| toml_edit::value(i64::from(performance.max_input_frames))),
        ),
        (
            "r8brain_extended_fft",
            (performance.extended_fft != default.extended_fft)
                .then(|| toml_edit::value(performance.extended_fft)),
        ),
        (
            "r8brain_filter_cache_size",
            (performance.filter_cache_size != default.filter_cache_size)
                .then(|| toml_edit::value(i64::from(performance.filter_cache_size))),
        ),
        (
            "r8brain_filter_bank_cache_size",
            (performance.filter_bank_cache_size != default.filter_bank_cache_size)
                .then(|| toml_edit::value(i64::from(performance.filter_bank_cache_size))),
        ),
    ]
}

const SOXR_FILTER_KEYS: [&str; 4] = [
    "soxr_precision",
    "soxr_passband_end",
    "soxr_phase",
    "soxr_double_precision",
];

const SOXR_KEYS: ParameterKeys = ParameterKeys {
    quality: "soxr_quality",
    filter: &SOXR_FILTER_KEYS,
};

fn write_soxr_changes(
    document: &mut toml_edit::DocumentMut,
    before: SoxrParameters,
    after: SoxrParameters,
) {
    let is_custom =
        |parameters: SoxrParameters| matches!(parameters.quality, SoxrQuality::Custom(_));
    write_changes(
        document,
        &SOXR_KEYS,
        (soxr_items(before).into(), is_custom(before)),
        (soxr_items(after).into(), is_custom(after)),
        before.quality != after.quality,
    );
}

/// As [`r8brain_items`], against the default quality's filter
fn soxr_items(parameters: SoxrParameters) -> [(&'static str, Option<toml_edit::Item>); 9] {
    let quality = parameters.quality;
    let preset = SoxrQuality::default().filter();
    let custom = match quality {
        SoxrQuality::Custom(filter) => Some(filter),
        _ => None,
    };
    let performance = parameters.performance;
    let default = SoxrPerformance::default();
    let whole = |number: u32| toml_edit::value(i64::from(number));
    [
        (
            "soxr_quality",
            (quality != SoxrQuality::default()).then(|| toml_edit::value(quality.name())),
        ),
        (
            "soxr_precision",
            custom
                .filter(|filter| filter.precision != preset.precision)
                .map(|filter| toml_edit::value(filter.precision)),
        ),
        (
            "soxr_passband_end",
            custom
                .filter(|filter| filter.passband_end != preset.passband_end)
                .map(|filter| toml_edit::value(filter.passband_end)),
        ),
        (
            "soxr_phase",
            custom
                .filter(|filter| filter.phase != preset.phase)
                .map(|filter| toml_edit::value(filter.phase.name())),
        ),
        (
            "soxr_double_precision",
            custom
                .filter(|filter| filter.double_precision != preset.double_precision)
                .map(|filter| toml_edit::value(filter.double_precision)),
        ),
        (
            "soxr_coef_table_kb",
            (performance.coef_table_kb != default.coef_table_kb)
                .then(|| whole(performance.coef_table_kb)),
        ),
        (
            "soxr_coef_interpolation",
            (performance.coef_interpolation != default.coef_interpolation)
                .then(|| toml_edit::value(performance.coef_interpolation.name())),
        ),
        (
            "soxr_min_dft_size",
            (performance.min_dft_size != default.min_dft_size)
                .then(|| whole(performance.min_dft_size)),
        ),
        (
            "soxr_large_dft_size",
            (performance.large_dft_size != default.large_dft_size)
                .then(|| whole(performance.large_dft_size)),
        ),
    ]
}

/// The settings directory and its `config.toml`:
///
/// ```toml
/// server = "https://music.example.com"
/// username = "alice"
/// volume = 0.8
/// song_cache_gib = 8
/// cover_cache_gib = 8
/// output_device = "coreaudio:BuiltInSpeakerDevice"
/// sample_rate = 48000 # Or "x2", "x4", "x8" or "x16", a multiple of each song's rate
/// exclusive_mode = true
/// trim_silence = true
/// crossfade_seconds = 5
/// crossfeed = true
/// crossfeed_preset = "custom"
/// crossfeed_cutoff_hz = 650
/// crossfeed_separation_db = 9.5
/// balance_db = -1.5 # Lowers the right channel; a positive value lowers the left
/// play_pause_fade = true
/// skip_instrumentals = true
/// resampling = "custom"
/// resampling_fft_size = 2048
/// resampling_window = "hann-2"
/// r8brain_quality = "custom"
/// r8brain_transition_band = 3.0
/// r8brain_attenuation = 136.45
/// r8brain_phase = "minimum"
/// r8brain_max_input_frames = 4096
/// r8brain_extended_fft = true
/// r8brain_whole_stepping = false
/// r8brain_filter_fractions = 2048
/// r8brain_filter_cache_size = 96
/// r8brain_filter_bank_cache_size = 12
/// soxr_quality = "custom"
/// soxr_precision = 24.0
/// soxr_passband_end = 0.95 # Of the Nyquist frequency
/// soxr_phase = "intermediate"
/// soxr_double_precision = false
/// soxr_coef_table_kb = 400
/// soxr_coef_interpolation = "auto"
/// soxr_min_dft_size = 1024
/// soxr_large_dft_size = 131072
/// output_format = "dsd"
/// dop_device = "coreaudio:SomeDacUid"
/// dsd_rate = 256
/// dsd_base_rate = "match"
/// dsd_fade_in = false
/// dsd_quality = "custom"
/// dsd_filter = "sinc"
/// dsd_sinc_length = 1024
/// dsd_window = "blackman-harris-2"
/// dsd_order = 7
/// dsd_noise_gain = 1.4
/// dsd_noise_band = 20000
/// dsd_optimized_zeros = true
/// dsd_input_level_db = 0.0
/// dsd_limiter_ceiling_db = 1.0
/// appearance = "dark"
/// lyrics_highlight = "line"
/// discord_presence = false
/// auto_update = false
/// imgbb_api_key = "..."
///
/// [[profiles]]
/// server = "https://music.example.com"
/// username = "alice"
///
/// [[profiles]]
/// server = "https://other.example.com"
/// username = "bob"
/// ```
#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    /// `~/.config/hiphile`, on every platform, so the files are in the same
    /// place wherever the app runs.
    pub fn user() -> Result<Self, Error> {
        let home = dirs::home_dir().ok_or(Error::NoHomeDir)?;
        Ok(Self::at(home.join(".config").join("hiphile")))
    }

    pub fn at(dir: impl AsRef<Path>) -> Self {
        Self {
            dir: dir.as_ref().to_owned(),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn config_path(&self) -> PathBuf {
        self.dir.join("config.toml")
    }

    /// `None` until both the server and the user are known.
    pub fn last_profile(&self) -> Result<Option<Profile>, Error> {
        let settings = self.read(config_rs::Environment::with_prefix(ENV_PREFIX))?;
        Ok(settings
            .server
            .zip(settings.username)
            .map(|(server, username)| Profile::new(server, username)))
    }

    /// Those signed in with and not forgotten since, each once, in the order first saved.
    /// Entries without a server and a username are left out.
    pub fn profiles(&self) -> Result<Vec<Profile>, Error> {
        let settings = self.read(config_rs::Environment::with_prefix(ENV_PREFIX))?;
        let saved = settings
            .profiles
            .and_then(Lenient::read)
            .unwrap_or_default();
        let mut profiles: Vec<Profile> = Vec::new();
        for saved in saved.into_iter().filter_map(Lenient::read) {
            let (Some(server), Some(username)) = (saved.server, saved.username) else {
                continue;
            };
            let profile = Profile::new(server, username);
            if !profiles.contains(&profile) {
                profiles.push(profile);
            }
        }
        Ok(profiles)
    }

    /// The password written by hand in the file, if it belongs to `profile`.
    /// It is only for the file's own `server` and `username`, so it is never
    /// sent to a server named by an option or the environment. The app never
    /// writes this key.
    pub fn password(&self, profile: &Profile) -> Result<Option<String>, Error> {
        let settings = self.read(
            config_rs::Environment::with_prefix(ENV_PREFIX).source(Some(Default::default())),
        )?;
        let same_profile = settings
            .server
            .zip(settings.username)
            .is_some_and(|(server, username)| Profile::new(server, username) == *profile);
        Ok(settings
            .password
            .filter(|password| same_profile && !password.is_empty()))
    }

    /// The playback volume from `0.0` to `1.0`, if one was saved.
    pub fn volume(&self) -> Result<Option<f32>, Error> {
        let settings = self.read(config_rs::Environment::with_prefix(ENV_PREFIX))?;
        Ok(settings
            .volume
            .filter(|volume| volume.is_finite())
            .map(|volume| volume.clamp(0.0, 1.0)))
    }

    /// How many bytes of songs may be kept on this computer, if the file says:
    /// `song_cache_gib`, in GiB, where `0` means no limit. A negative number is an error, not a
    /// reason to keep the default.
    pub fn song_cache_capacity(&self) -> Result<Option<u64>, Error> {
        let settings = self.read(config_rs::Environment::with_prefix(ENV_PREFIX))?;
        self.cache_bytes("song_cache_gib", settings.song_cache_gib)
    }

    pub fn song_cache_gib(&self) -> Result<Option<f64>, Error> {
        let settings = self.read(config_rs::Environment::with_prefix(ENV_PREFIX))?;
        self.cache_bytes("song_cache_gib", settings.song_cache_gib)?;
        Ok(settings.song_cache_gib)
    }

    /// How many bytes of covers may be kept on this computer, if the file says:
    /// `cover_cache_gib`, in GiB, where `0` means no limit. A negative number is an error, not a
    /// reason to keep the default.
    pub fn cover_cache_capacity(&self) -> Result<Option<u64>, Error> {
        let settings = self.read(config_rs::Environment::with_prefix(ENV_PREFIX))?;
        self.cache_bytes("cover_cache_gib", settings.cover_cache_gib)
    }

    pub fn cover_cache_gib(&self) -> Result<Option<f64>, Error> {
        let settings = self.read(config_rs::Environment::with_prefix(ENV_PREFIX))?;
        self.cache_bytes("cover_cache_gib", settings.cover_cache_gib)?;
        Ok(settings.cover_cache_gib)
    }

    /// The id of the output device to play on, if one was chosen rather than the system's
    /// default.
    pub fn output_device(&self) -> Result<Option<String>, Error> {
        let settings = self.read(config_rs::Environment::with_prefix(ENV_PREFIX))?;
        Ok(settings.output_device)
    }

    pub fn sample_rate(&self) -> Result<SampleRate, Error> {
        let settings = self.read(config_rs::Environment::with_prefix(ENV_PREFIX))?;
        self.parse_sample_rate(&settings)
    }

    fn parse_sample_rate(&self, settings: &Settings) -> Result<SampleRate, Error> {
        settings
            .sample_rate
            .as_deref()
            .map_or(Ok(SampleRate::MatchSong), |text| {
                SampleRate::parse(text).ok_or_else(|| {
                    self.invalid("sample_rate", r#"a rate in Hz, "x2", "x4", "x8" or "x16""#)
                })
            })
    }

    /// Whether to keep other apps off the output device while playing.
    pub fn exclusive_mode(&self) -> Result<bool, Error> {
        let settings = self.read(config_rs::Environment::with_prefix(ENV_PREFIX))?;
        Ok(settings.exclusive_mode.unwrap_or(false))
    }

    pub fn trim_silence(&self) -> Result<bool, Error> {
        let settings = self.read(config_rs::Environment::with_prefix(ENV_PREFIX))?;
        Ok(settings.trim_silence.unwrap_or(false))
    }

    pub fn play_pause_fade(&self) -> Result<bool, Error> {
        let settings = self.read(config_rs::Environment::with_prefix(ENV_PREFIX))?;
        Ok(settings.play_pause_fade.unwrap_or(false))
    }

    /// How long songs overlap, up to [`LONGEST_CROSSFADE_SECONDS`]; `0` joins them.
    pub fn crossfade_seconds(&self) -> Result<u32, Error> {
        let settings = self.read(config_rs::Environment::with_prefix(ENV_PREFIX))?;
        let seconds = settings.crossfade_seconds.unwrap_or(0);
        if seconds > LONGEST_CROSSFADE_SECONDS {
            return Err(self.invalid("crossfade_seconds", "whole seconds from 0 to 12"));
        }
        Ok(seconds)
    }

    /// Off unless the file says otherwise. The preset is read either way, so that turning
    /// crossfeed on comes back to it. The cutoff and separation are only read for `custom`, and
    /// default to those of `bs2b`.
    pub fn crossfeed(&self) -> Result<Crossfeed, Error> {
        let settings = self.read(config_rs::Environment::with_prefix(ENV_PREFIX))?;
        self.parse_crossfeed(&settings)
    }

    fn parse_crossfeed(&self, settings: &Settings) -> Result<Crossfeed, Error> {
        let preset = match settings.crossfeed_preset.as_deref() {
            None | Some("bs2b") => CrossfeedPreset::Bs2b,
            Some("chu-moy") => CrossfeedPreset::ChuMoy,
            Some("jan-meier") => CrossfeedPreset::JanMeier,
            Some("custom") => CrossfeedPreset::Custom(self.parse_crossfeed_filter(settings)?),
            Some(_) => {
                return Err(self.invalid(
                    "crossfeed_preset",
                    "\"bs2b\", \"chu-moy\", \"jan-meier\" or \"custom\"",
                ));
            }
        };
        Ok(Crossfeed {
            is_on: settings.crossfeed.unwrap_or(false),
            preset,
        })
    }

    fn parse_crossfeed_filter(&self, settings: &Settings) -> Result<CrossfeedFilter, Error> {
        let bs2b = CrossfeedPreset::Bs2b.filter();
        let cutoff_hz = settings.crossfeed_cutoff_hz.unwrap_or(bs2b.cutoff_hz);
        if !CROSSFEED_CUTOFFS_HZ.contains(&cutoff_hz) {
            return Err(self.invalid("crossfeed_cutoff_hz", "a whole number from 300 to 2000"));
        }
        // Tenths are what plays and what Settings shows
        let separation_db = settings
            .crossfeed_separation_db
            .map_or(bs2b.separation_db, |db| (db * 10.0).round() / 10.0);
        if !CROSSFEED_SEPARATIONS_DB.contains(&separation_db) {
            return Err(self.invalid("crossfeed_separation_db", "a number from 1 to 15"));
        }
        Ok(CrossfeedFilter {
            cutoff_hz,
            separation_db,
        })
    }

    /// `0` unless the file says otherwise. See [`BALANCE_DBS`].
    pub fn balance_db(&self) -> Result<f64, Error> {
        let settings = self.read(config_rs::Environment::with_prefix(ENV_PREFIX))?;
        let db = settings.balance_db.unwrap_or(0.0);
        if !BALANCE_DBS.contains(&db) {
            return Err(self.invalid("balance_db", "a number from -12 to 12"));
        }
        Ok(db)
    }

    /// Medium unless the file says otherwise. The FFT size and window are only read for
    /// `custom`, and default to those of medium. The `r8brain_` keys are only read for `r8brain`,
    /// and those that decide the sound only for its `custom`, defaulting to those of its `high`.
    /// The `soxr_` keys are read in the same way for `soxr`, defaulting to its `very-high`.
    pub fn resampling_quality(&self) -> Result<ResamplingQuality, Error> {
        let settings = self.read(config_rs::Environment::with_prefix(ENV_PREFIX))?;
        self.parse_resampling_quality(&settings)
    }

    /// Whatever resampling is in use, so that it can come back to these.
    pub fn r8brain_parameters(&self) -> Result<R8brainParameters, Error> {
        let settings = self.read(config_rs::Environment::with_prefix(ENV_PREFIX))?;
        self.parse_r8brain_parameters(&settings)
    }

    /// Whatever resampling is in use, so that it can come back to these.
    pub fn soxr_parameters(&self) -> Result<SoxrParameters, Error> {
        let settings = self.read(config_rs::Environment::with_prefix(ENV_PREFIX))?;
        self.parse_soxr_parameters(&settings)
    }

    /// Off unless the file says otherwise. The parameters are only read for
    /// the custom quality, and default to those of high.
    pub fn dsd_output(&self) -> Result<DsdOutput, Error> {
        let settings = self.read(config_rs::Environment::with_prefix(ENV_PREFIX))?;
        self.parse_dsd_output(&settings)
    }

    /// Light or dark, if one was chosen rather than following the system. `system` in
    /// `HIPHILE_APPEARANCE` sets a choice in the file aside.
    pub fn appearance(&self) -> Result<Option<Appearance>, Error> {
        let settings = self.read(config_rs::Environment::with_prefix(ENV_PREFIX))?;
        self.parse_appearance(settings.appearance.as_deref())
    }

    /// By word unless the file says otherwise.
    pub fn lyrics_highlight(&self) -> Result<LyricsHighlight, Error> {
        let settings = self.read(config_rs::Environment::with_prefix(ENV_PREFIX))?;
        self.parse_lyrics_highlight(settings.lyrics_highlight.as_deref())
    }

    /// Whether kanji can be searched for by romaji, which takes dictionaries that are
    /// downloaded for it. On unless the file says otherwise.
    pub fn romaji_search(&self) -> Result<bool, Error> {
        let settings = self.read(config_rs::Environment::with_prefix(ENV_PREFIX))?;
        Ok(settings.romaji_search.unwrap_or(true))
    }

    pub fn skip_instrumentals(&self) -> Result<bool, Error> {
        let settings = self.read(config_rs::Environment::with_prefix(ENV_PREFIX))?;
        Ok(settings.skip_instrumentals.unwrap_or(false))
    }

    /// On unless the file says otherwise.
    pub fn discord_presence(&self) -> Result<bool, Error> {
        let settings = self.read(config_rs::Environment::with_prefix(ENV_PREFIX))?;
        Ok(settings.discord_presence.unwrap_or(true))
    }

    /// On unless the file says otherwise.
    pub fn auto_update(&self) -> Result<bool, Error> {
        let settings = self.read(config_rs::Environment::with_prefix(ENV_PREFIX))?;
        Ok(settings.auto_update.unwrap_or(true))
    }

    pub fn imgbb_api_key(&self) -> Result<Option<String>, Error> {
        let settings = self.read(config_rs::Environment::with_prefix(ENV_PREFIX))?;
        Ok(usable_key(settings.imgbb_api_key))
    }

    fn parse_appearance(&self, value: Option<&str>) -> Result<Option<Appearance>, Error> {
        match value {
            None | Some("system") => Ok(None),
            Some("light") => Ok(Some(Appearance::Light)),
            Some("dark") => Ok(Some(Appearance::Dark)),
            Some(_) => Err(Error::InvalidSetting {
                path: self.config_path(),
                key: "appearance",
                expected: "\"system\", \"light\" or \"dark\"",
            }),
        }
    }

    fn parse_resampling_quality(&self, settings: &Settings) -> Result<ResamplingQuality, Error> {
        let invalid = |key, expected| Error::InvalidSetting {
            path: self.config_path(),
            key,
            expected,
        };
        match settings.resampling.as_deref() {
            None | Some("medium") => Ok(ResamplingQuality::Medium),
            Some("high") => Ok(ResamplingQuality::High),
            Some("max") => Ok(ResamplingQuality::Max),
            Some("r8brain") => self
                .parse_r8brain_parameters(settings)
                .map(ResamplingQuality::R8brain),
            Some("soxr") => self
                .parse_soxr_parameters(settings)
                .map(ResamplingQuality::Soxr),
            Some("custom") => {
                let medium = ResamplingQuality::Medium;
                let fft_size = settings.resampling_fft_size.unwrap_or(medium.fft_size());
                if !RESAMPLING_FFT_SIZES.contains(&fft_size) {
                    return Err(invalid(
                        "resampling_fft_size",
                        "a power of two from 256 to 16384",
                    ));
                }
                let window = match settings.resampling_window.as_deref() {
                    None => medium.window(),
                    Some(name) => ResamplingWindow::ALL
                        .into_iter()
                        .find(|window| window.name() == name)
                        .ok_or_else(|| {
                            invalid(
                                "resampling_window",
                                "\"hann\", \"hann-2\", \"blackman\", \"blackman-2\", \
                                 \"blackman-harris\" or \"blackman-harris-2\"",
                            )
                        })?,
                };
                Ok(ResamplingQuality::Custom { fft_size, window })
            }
            Some(_) => Err(invalid(
                "resampling",
                "\"medium\", \"high\", \"max\", \"custom\", \"r8brain\" or \"soxr\"",
            )),
        }
    }

    fn parse_soxr_parameters(&self, settings: &Settings) -> Result<SoxrParameters, Error> {
        let quality = match settings.soxr_quality.as_deref() {
            Some("low") => SoxrQuality::Low,
            Some("medium") => SoxrQuality::Medium,
            Some("high") => SoxrQuality::High,
            None | Some("very-high") => SoxrQuality::VeryHigh,
            Some("custom") => SoxrQuality::Custom(self.parse_soxr_filter(settings)?),
            Some(_) => {
                return Err(self.invalid(
                    "soxr_quality",
                    "\"low\", \"medium\", \"high\", \"very-high\" or \"custom\"",
                ));
            }
        };
        Ok(SoxrParameters {
            quality,
            performance: self.parse_soxr_performance(settings)?,
        })
    }

    fn parse_soxr_filter(&self, settings: &Settings) -> Result<SoxrFilter, Error> {
        let preset = SoxrQuality::default().filter();
        let precision = settings.soxr_precision.unwrap_or(preset.precision);
        if !SOXR_PRECISIONS.contains(&precision) {
            return Err(self.invalid("soxr_precision", "a number of bits from 15 to 33"));
        }
        let passband_end = settings.soxr_passband_end.unwrap_or(preset.passband_end);
        if !SOXR_PASSBAND_ENDS.contains(&passband_end) {
            return Err(self.invalid(
                "soxr_passband_end",
                "a fraction of the Nyquist frequency from 0.5 to 0.998",
            ));
        }
        let phase = match settings.soxr_phase.as_deref() {
            None | Some("linear") => SoxrPhase::Linear,
            Some("intermediate") => SoxrPhase::Intermediate,
            Some("minimum") => SoxrPhase::Minimum,
            Some(_) => {
                return Err(
                    self.invalid("soxr_phase", "\"linear\", \"intermediate\" or \"minimum\"")
                );
            }
        };
        Ok(SoxrFilter {
            precision,
            passband_end,
            phase,
            double_precision: settings
                .soxr_double_precision
                .unwrap_or(preset.double_precision),
        })
    }

    fn parse_soxr_performance(&self, settings: &Settings) -> Result<SoxrPerformance, Error> {
        let default = SoxrPerformance::default();
        let coef_table_kb = settings.soxr_coef_table_kb.unwrap_or(default.coef_table_kb);
        if !SOXR_COEF_TABLE_KBS.contains(&coef_table_kb) {
            return Err(self.invalid("soxr_coef_table_kb", "a whole number from 1 to 65536"));
        }
        let coef_interpolation = match settings.soxr_coef_interpolation.as_deref() {
            None | Some("auto") => SoxrInterpolation::Auto,
            Some("linear") => SoxrInterpolation::Linear,
            Some("cubic") => SoxrInterpolation::Cubic,
            Some(_) => {
                return Err(self.invalid(
                    "soxr_coef_interpolation",
                    "\"auto\", \"linear\" or \"cubic\"",
                ));
            }
        };
        let min_dft_size = settings.soxr_min_dft_size.unwrap_or(default.min_dft_size);
        if !SOXR_MIN_DFT_SIZES.contains(&min_dft_size) {
            return Err(self.invalid("soxr_min_dft_size", "a power of two from 256 to 32768"));
        }
        let large_dft_size = settings
            .soxr_large_dft_size
            .unwrap_or(default.large_dft_size);
        if !SOXR_LARGE_DFT_SIZES.contains(&large_dft_size) {
            return Err(self.invalid("soxr_large_dft_size", "a power of two from 256 to 1048576"));
        }
        Ok(SoxrPerformance {
            coef_table_kb,
            coef_interpolation,
            min_dft_size,
            large_dft_size,
        })
    }

    fn parse_r8brain_parameters(&self, settings: &Settings) -> Result<R8brainParameters, Error> {
        let quality = match settings.r8brain_quality.as_deref() {
            Some("low") => R8brainQuality::Low,
            Some("medium") => R8brainQuality::Medium,
            None | Some("high") => R8brainQuality::High,
            Some("max") => R8brainQuality::Max,
            Some("custom") => R8brainQuality::Custom(self.parse_r8brain_filter(settings)?),
            Some(_) => {
                return Err(self.invalid(
                    "r8brain_quality",
                    "\"low\", \"medium\", \"high\", \"max\" or \"custom\"",
                ));
            }
        };
        Ok(R8brainParameters {
            quality,
            performance: self.parse_r8brain_performance(settings)?,
        })
    }

    fn parse_r8brain_filter(&self, settings: &Settings) -> Result<R8brainFilter, Error> {
        let high = R8brainQuality::High.filter();
        let transition_band = settings
            .r8brain_transition_band
            .unwrap_or(high.transition_band);
        if !R8BRAIN_TRANSITION_BANDS.contains(&transition_band) {
            return Err(self.invalid("r8brain_transition_band", "a number from 0.5 to 45"));
        }
        let attenuation = settings.r8brain_attenuation.unwrap_or(high.attenuation);
        if !R8BRAIN_ATTENUATIONS.contains(&attenuation) {
            return Err(self.invalid("r8brain_attenuation", "a number from 49 to 218"));
        }
        let phase = match settings.r8brain_phase.as_deref() {
            None | Some("linear") => R8brainPhase::Linear,
            Some("minimum") => R8brainPhase::Minimum,
            Some(_) => return Err(self.invalid("r8brain_phase", "\"linear\" or \"minimum\"")),
        };
        let filter_fractions = settings.r8brain_filter_fractions;
        if filter_fractions.is_some_and(|fractions| !R8BRAIN_FILTER_FRACTIONS.contains(&fractions))
        {
            return Err(self.invalid("r8brain_filter_fractions", "a whole number from 5 to 65536"));
        }
        Ok(R8brainFilter {
            transition_band,
            attenuation,
            phase,
            whole_stepping: settings
                .r8brain_whole_stepping
                .unwrap_or(high.whole_stepping),
            filter_fractions,
        })
    }

    fn parse_r8brain_performance(&self, settings: &Settings) -> Result<R8brainPerformance, Error> {
        let default = R8brainPerformance::default();
        let max_input_frames = settings
            .r8brain_max_input_frames
            .unwrap_or(default.max_input_frames);
        if !R8BRAIN_MAX_INPUT_FRAMES.contains(&max_input_frames) {
            return Err(self.invalid(
                "r8brain_max_input_frames",
                "a whole number from 1 to 1048576",
            ));
        }
        let filter_cache_size = settings
            .r8brain_filter_cache_size
            .unwrap_or(default.filter_cache_size);
        if !R8BRAIN_CACHE_SIZES.contains(&filter_cache_size) {
            return Err(self.invalid("r8brain_filter_cache_size", "a whole number from 2 to 4096"));
        }
        let filter_bank_cache_size = settings
            .r8brain_filter_bank_cache_size
            .unwrap_or(default.filter_bank_cache_size);
        if !R8BRAIN_CACHE_SIZES.contains(&filter_bank_cache_size) {
            return Err(self.invalid(
                "r8brain_filter_bank_cache_size",
                "a whole number from 2 to 4096",
            ));
        }
        Ok(R8brainPerformance {
            max_input_frames,
            extended_fft: settings
                .r8brain_extended_fft
                .unwrap_or(default.extended_fft),
            filter_cache_size,
            filter_bank_cache_size,
        })
    }

    fn parse_dsd_output(&self, settings: &Settings) -> Result<DsdOutput, Error> {
        let is_on = match settings.output_format.as_deref() {
            None | Some("pcm") => false,
            Some("dsd") => true,
            Some(_) => return Err(self.invalid("output_format", "\"pcm\" or \"dsd\"")),
        };
        let rate = settings
            .dsd_rate
            .map(|multiple| {
                DsdRate::ALL
                    .into_iter()
                    .find(|rate| rate.multiple() == multiple)
                    .ok_or_else(|| self.invalid("dsd_rate", "64, 128 or 256"))
            })
            .transpose()?;
        let matches_song_family = match settings.dsd_base_rate.as_deref() {
            None | Some("44100") => false,
            Some("match") => true,
            Some(_) => return Err(self.invalid("dsd_base_rate", "44100 or \"match\"")),
        };
        Ok(DsdOutput {
            is_on,
            device: settings.dop_device.clone().filter(|id| !id.is_empty()),
            rate,
            matches_song_family,
            quality: self.parse_dsd_quality(settings)?,
            fades_in: settings.dsd_fade_in.unwrap_or(true),
        })
    }

    fn parse_dsd_quality(&self, settings: &Settings) -> Result<DsdQuality, Error> {
        match settings.dsd_quality.as_deref() {
            Some("low") => Ok(DsdQuality::Low),
            Some("medium") => Ok(DsdQuality::Medium),
            None | Some("high") => Ok(DsdQuality::High),
            Some("max") => Ok(DsdQuality::Max),
            Some("custom") => self.parse_dsd_parameters(settings).map(DsdQuality::Custom),
            Some(_) => Err(self.invalid(
                "dsd_quality",
                "\"low\", \"medium\", \"high\", \"max\" or \"custom\"",
            )),
        }
    }

    fn parse_dsd_parameters(&self, settings: &Settings) -> Result<DsdParameters, Error> {
        let high = DsdQuality::High.parameters();
        let order = settings.dsd_order.unwrap_or(high.order);
        if !DSD_ORDERS.contains(&order) {
            return Err(self.invalid("dsd_order", "3, 5, 7 or 9"));
        }
        let noise_gain = settings.dsd_noise_gain.unwrap_or(high.noise_gain);
        if !DSD_NOISE_GAINS.contains(&noise_gain) {
            return Err(self.invalid("dsd_noise_gain", "a number from 1.2 to 1.5"));
        }
        let wide_noise_band = match settings.dsd_noise_band {
            None | Some(20_000) => false,
            Some(48_000) => true,
            Some(_) => return Err(self.invalid("dsd_noise_band", "20000 or 48000")),
        };
        let dither_db = settings.dsd_dither_db;
        if dither_db.is_some_and(|dither_db| !DSD_DITHER_DBS.contains(&dither_db)) {
            return Err(self.invalid("dsd_dither_db", "a number from -160 to -20"));
        }
        let input_level_db = settings.dsd_input_level_db.unwrap_or(high.input_level_db);
        if !DSD_INPUT_LEVEL_DBS.contains(&input_level_db) {
            return Err(self.invalid("dsd_input_level_db", "a number from -24 to 6"));
        }
        let limiter_ceiling_db = match settings.dsd_limiter_ceiling_db.as_deref() {
            None => high.limiter_ceiling_db,
            Some("off") => None,
            Some(text) => Some(
                text.parse()
                    .ok()
                    .filter(|ceiling| DSD_CEILING_DBS.contains(ceiling))
                    .ok_or_else(|| {
                        self.invalid("dsd_limiter_ceiling_db", "\"off\" or a number from -6 to 3")
                    })?,
            ),
        };
        Ok(DsdParameters {
            filter: self.parse_dsd_filter(settings)?,
            order,
            noise_gain,
            wide_noise_band,
            optimized_zeros: settings.dsd_optimized_zeros.unwrap_or(high.optimized_zeros),
            dither_db,
            input_level_db,
            limiter_ceiling_db,
        })
    }

    fn parse_dsd_filter(&self, settings: &Settings) -> Result<DsdFilter, Error> {
        let window = match settings.dsd_window.as_deref() {
            None => ResamplingWindow::BlackmanHarris2,
            Some(name) => ResamplingWindow::ALL
                .into_iter()
                .find(|window| window.name() == name)
                .ok_or_else(|| {
                    self.invalid(
                        "dsd_window",
                        "\"hann\", \"hann-2\", \"blackman\", \"blackman-2\", \
                         \"blackman-harris\" or \"blackman-harris-2\"",
                    )
                })?,
        };
        match settings.dsd_filter.as_deref() {
            None | Some("sinc") => {
                let length = settings.dsd_sinc_length.unwrap_or(1024);
                if !DSD_SINC_LENGTHS.contains(&length) {
                    return Err(self.invalid("dsd_sinc_length", "256, 512, 1024 or 2048"));
                }
                let cutoff = settings.dsd_cutoff;
                if cutoff.is_some_and(|cutoff| !DSD_CUTOFFS.contains(&cutoff)) {
                    return Err(self.invalid("dsd_cutoff", "a number from 0.5 to 1"));
                }
                Ok(DsdFilter::Sinc {
                    length,
                    window,
                    cutoff,
                })
            }
            Some("fft") => {
                let size = settings.dsd_fft_size.unwrap_or(4096);
                if !RESAMPLING_FFT_SIZES.contains(&size) {
                    return Err(self.invalid("dsd_fft_size", "a power of two from 256 to 16384"));
                }
                Ok(DsdFilter::Fft { size, window })
            }
            Some(_) => Err(self.invalid("dsd_filter", "\"sinc\" or \"fft\"")),
        }
    }

    fn invalid(&self, key: &'static str, expected: &'static str) -> Error {
        Error::InvalidSetting {
            path: self.config_path(),
            key,
            expected,
        }
    }

    fn parse_lyrics_highlight(&self, value: Option<&str>) -> Result<LyricsHighlight, Error> {
        match value {
            None => Ok(LyricsHighlight::default()),
            Some("word") => Ok(LyricsHighlight::ByWord),
            Some("line") => Ok(LyricsHighlight::ByLine),
            Some(_) => Err(Error::InvalidSetting {
                path: self.config_path(),
                key: "lyrics_highlight",
                expected: "\"word\" or \"line\"",
            }),
        }
    }

    fn cache_bytes(&self, key: &'static str, gib: Option<f64>) -> Result<Option<u64>, Error> {
        let Some(gib) = gib else {
            return Ok(None);
        };
        cache_capacity_of(gib)
            .map(Some)
            .ok_or_else(|| Error::InvalidSetting {
                path: self.config_path(),
                key,
                expected: "a number of GiB, 0 or more",
            })
    }

    fn read(&self, environment: config_rs::Environment) -> Result<Settings, Error> {
        let path = self.config_path();
        let read_error = |source| Error::Read {
            path: path.clone(),
            source,
        };
        config_rs::Config::builder()
            .add_source(
                config_rs::File::from(path.as_path())
                    .format(config_rs::FileFormat::Toml)
                    .required(false),
            )
            .add_source(environment)
            .build()
            .and_then(config_rs::Config::try_deserialize)
            .map_err(read_error)
    }

    /// Records `profile` in `config.toml` as the last one, and among the
    /// profiles unless it is there, keeping the file's other keys and comments.
    /// A different profile removes `password`, which belonged to the one it
    /// replaces: that one comes back with it, for the password to be kept
    /// elsewhere.
    pub fn save_profile(&self, profile: &Profile) -> Result<Option<(Profile, String)>, Error> {
        let mut replaced = None;
        self.edit(|document| {
            if !is_profile(document.as_table(), profile) {
                let saved = |key| document.get(key).and_then(toml_edit::Item::as_str);
                let password = saved("password").filter(|password| !password.is_empty());
                replaced = saved("server").zip(saved("username")).zip(password).map(
                    |((server, username), password)| {
                        (Profile::new(server, username), password.to_owned())
                    },
                );
                document.remove("password");
            }
            document["server"] = toml_edit::value(profile.server());
            document["username"] = toml_edit::value(profile.username());
            let profiles = document
                .entry("profiles")
                .or_insert(toml_edit::Item::ArrayOfTables(Default::default()));
            match ProfileList::of(profiles) {
                Some(profiles) if !profiles.has(profile) => profiles.push(profile),
                _ => {}
            }
        })?;
        Ok(replaced)
    }

    /// Takes `profile` off the profiles. It stays the last one, if it is.
    pub fn forget_profile(&self, profile: &Profile) -> Result<(), Error> {
        self.edit(|document| {
            let Some(mut profiles) = document.get_mut("profiles").and_then(ProfileList::of) else {
                return;
            };
            profiles.remove(profile);
            if profiles.is_empty() {
                document.remove("profiles");
            }
        })
    }

    pub fn save_volume(&self, volume: f32) -> Result<(), Error> {
        // Two decimals are plenty, and keep the file readable
        let volume = (f64::from(volume.clamp(0.0, 1.0)) * 100.0).round() / 100.0;
        self.edit(|document| document["volume"] = toml_edit::value(volume))
    }

    /// `None` removes the key, which means the system's default device.
    pub fn save_output_device(&self, device: Option<&str>) -> Result<(), Error> {
        self.edit(|document| match device {
            Some(device) => document["output_device"] = toml_edit::value(device),
            None => {
                document.remove("output_device");
            }
        })
    }

    pub fn save_sample_rate(&self, sample_rate: SampleRate) -> Result<(), Error> {
        self.edit(|document| match sample_rate {
            SampleRate::MatchSong => {
                document.remove("sample_rate");
            }
            SampleRate::Fixed(rate) => document["sample_rate"] = toml_edit::value(i64::from(rate)),
            SampleRate::SongMultiple(_) => {
                document["sample_rate"] = toml_edit::value(sample_rate.text());
            }
        })
    }

    pub fn save_exclusive_mode(&self, is_on: bool) -> Result<(), Error> {
        self.edit(|document| {
            if is_on {
                document["exclusive_mode"] = toml_edit::value(true);
            } else {
                document.remove("exclusive_mode");
            }
        })
    }

    pub fn save_trim_silence(&self, is_on: bool) -> Result<(), Error> {
        self.edit(|document| {
            if is_on {
                document["trim_silence"] = toml_edit::value(true);
            } else {
                document.remove("trim_silence");
            }
        })
    }

    pub fn save_play_pause_fade(&self, is_on: bool) -> Result<(), Error> {
        self.edit(|document| {
            if is_on {
                document["play_pause_fade"] = toml_edit::value(true);
            } else {
                document.remove("play_pause_fade");
            }
        })
    }

    pub fn save_skip_instrumentals(&self, is_on: bool) -> Result<(), Error> {
        self.edit(|document| {
            if is_on {
                document["skip_instrumentals"] = toml_edit::value(true);
            } else {
                document.remove("skip_instrumentals");
            }
        })
    }

    pub fn save_crossfade_seconds(&self, seconds: u32) -> Result<(), Error> {
        self.edit(|document| {
            if seconds == 0 {
                document.remove("crossfade_seconds");
            } else {
                document["crossfade_seconds"] = toml_edit::value(i64::from(seconds));
            }
        })
    }

    /// Writes only what differs from `before`, the values in use until now: the rest may have
    /// come from the environment.
    pub fn save_crossfeed_changes(&self, before: Crossfeed, after: Crossfeed) -> Result<(), Error> {
        self.edit(|document| write_crossfeed_changes(document, before, after))
    }

    pub fn save_balance_db(&self, db: f64) -> Result<(), Error> {
        self.edit(|document| {
            if db == 0.0 {
                document.remove("balance_db");
            } else {
                document["balance_db"] = toml_edit::value(db);
            }
        })
    }

    pub fn save_resampling_quality(&self, quality: ResamplingQuality) -> Result<(), Error> {
        self.edit(|document| {
            document["resampling"] = toml_edit::value(quality.name());
            if let ResamplingQuality::Custom { fft_size, window } = quality {
                document["resampling_fft_size"] = toml_edit::value(i64::from(fft_size));
                document["resampling_window"] = toml_edit::value(window.name());
            } else {
                document.remove("resampling_fft_size");
                document.remove("resampling_window");
            }
        })
    }

    pub fn save_r8brain_changes(
        &self,
        before: R8brainParameters,
        after: R8brainParameters,
    ) -> Result<(), Error> {
        self.edit(|document| write_r8brain_changes(document, before, after))
    }

    pub fn save_soxr_changes(
        &self,
        before: SoxrParameters,
        after: SoxrParameters,
    ) -> Result<(), Error> {
        self.edit(|document| write_soxr_changes(document, before, after))
    }

    /// Writes only what the quality uses, so that a key left over from another
    /// one cannot come back unnoticed.
    /// Writes only what differs from `before`, the values in use until now:
    /// the rest may have come from the environment, or from a file that could
    /// not be read, and turning DSD on is not to be written behind anyone's back.
    pub fn save_dsd_changes(&self, before: &DsdOutput, after: &DsdOutput) -> Result<(), Error> {
        self.edit(|document| {
            if before.is_on != after.is_on {
                document["output_format"] =
                    toml_edit::value(if after.is_on { "dsd" } else { "pcm" });
            }
            if before.device != after.device {
                match &after.device {
                    Some(device) => document["dop_device"] = toml_edit::value(device.as_str()),
                    None => {
                        document.remove("dop_device");
                    }
                }
            }
            if before.rate != after.rate {
                match after.rate {
                    Some(rate) => {
                        document["dsd_rate"] = toml_edit::value(i64::from(rate.multiple()));
                    }
                    None => {
                        document.remove("dsd_rate");
                    }
                }
            }
            if before.matches_song_family != after.matches_song_family {
                if after.matches_song_family {
                    document["dsd_base_rate"] = toml_edit::value("match");
                } else {
                    document.remove("dsd_base_rate");
                }
            }
            if before.fades_in != after.fades_in {
                if after.fades_in {
                    document.remove("dsd_fade_in");
                } else {
                    document["dsd_fade_in"] = toml_edit::value(false);
                }
            }
            if before.quality != after.quality {
                document["dsd_quality"] = toml_edit::value(after.quality.name());
                let custom = |quality: &DsdQuality| match quality {
                    DsdQuality::Custom(parameters) => Some(*parameters),
                    _ => None,
                };
                save_dsd_parameters(
                    document,
                    custom(&before.quality).as_ref(),
                    custom(&after.quality).as_ref(),
                );
            }
        })
    }

    /// `None` removes the key, which means the default amount. A negative number would make
    /// the setting unreadable, so callers keep to `0` or more.
    pub fn save_song_cache_gib(&self, gib: Option<f64>) -> Result<(), Error> {
        self.edit(|document| match gib {
            Some(gib) => document["song_cache_gib"] = toml_edit::value(gib),
            None => {
                document.remove("song_cache_gib");
            }
        })
    }

    /// Records how many GiB of covers to keep, like
    /// [`Self::save_song_cache_gib`].
    pub fn save_cover_cache_gib(&self, gib: Option<f64>) -> Result<(), Error> {
        self.edit(|document| match gib {
            Some(gib) => document["cover_cache_gib"] = toml_edit::value(gib),
            None => {
                document.remove("cover_cache_gib");
            }
        })
    }

    /// `None` removes the key, which means following the system.
    pub fn save_appearance(&self, appearance: Option<Appearance>) -> Result<(), Error> {
        self.edit(|document| match appearance {
            Some(Appearance::Light) => document["appearance"] = toml_edit::value("light"),
            Some(Appearance::Dark) => document["appearance"] = toml_edit::value("dark"),
            None => {
                document.remove("appearance");
            }
        })
    }

    pub fn save_lyrics_highlight(&self, highlight: LyricsHighlight) -> Result<(), Error> {
        let value = match highlight {
            LyricsHighlight::ByWord => "word",
            LyricsHighlight::ByLine => "line",
        };
        self.edit(|document| document["lyrics_highlight"] = toml_edit::value(value))
    }

    pub fn save_romaji_search(&self, is_on: bool) -> Result<(), Error> {
        self.edit(|document| document["romaji_search"] = toml_edit::value(is_on))
    }

    pub fn save_discord_presence(&self, is_on: bool) -> Result<(), Error> {
        self.edit(|document| document["discord_presence"] = toml_edit::value(is_on))
    }

    pub fn save_auto_update(&self, is_on: bool) -> Result<(), Error> {
        self.edit(|document| document["auto_update"] = toml_edit::value(is_on))
    }

    /// `None` removes the key, and with it the covers on the Discord status.
    pub fn save_imgbb_api_key(&self, key: Option<&str>) -> Result<(), Error> {
        self.edit(|document| match key {
            Some(key) => document["imgbb_api_key"] = toml_edit::value(key),
            None => {
                document.remove("imgbb_api_key");
            }
        })
    }

    fn edit(&self, change: impl FnOnce(&mut toml_edit::DocumentMut)) -> Result<(), Error> {
        let io_error = |path: &Path| {
            let path = path.to_owned();
            move |source| Error::Io { path, source }
        };
        let _editing = EDITING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let path = self.config_path();
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(source) => return Err(Error::Io { path, source }),
        };
        let mut document: toml_edit::DocumentMut =
            text.parse().map_err(|source| Error::Unparsable {
                path: path.clone(),
                source,
            })?;
        change(&mut document);

        std::fs::create_dir_all(&self.dir).map_err(io_error(&self.dir))?;
        // Write then rename, so a crash never leaves a half-written file
        let temporary = path.with_extension(format!(
            "toml.{}.{}.tmp",
            std::process::id(),
            TEMPORARY.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&temporary, document.to_string()).map_err(io_error(&temporary))?;
        std::fs::rename(&temporary, &path).map_err(io_error(&path))
    }
}

/// `server` with its scheme and host in lowercase, and without slashes at its end.
fn server_address(server: &str) -> String {
    let server = server.trim().trim_end_matches('/');
    let Some((scheme, rest)) = server.split_once("://") else {
        return server.to_owned();
    };
    let (host, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    format!(
        "{}://{}{path}",
        scheme.to_ascii_lowercase(),
        host.to_ascii_lowercase()
    )
}

/// The profiles in `config.toml`, changed in the form they are written in.
enum ProfileList<'a> {
    Tables(&'a mut toml_edit::ArrayOfTables),
    Inline(&'a mut toml_edit::Array),
}

impl<'a> ProfileList<'a> {
    /// `None` when `item` is neither, which is read as no profiles and left as it is.
    fn of(item: &'a mut toml_edit::Item) -> Option<Self> {
        match item {
            toml_edit::Item::ArrayOfTables(tables) => Some(Self::Tables(tables)),
            toml_edit::Item::Value(toml_edit::Value::Array(array)) => Some(Self::Inline(array)),
            _ => None,
        }
    }

    fn has(&self, profile: &Profile) -> bool {
        match self {
            Self::Tables(tables) => tables.iter().any(|table| is_profile(table, profile)),
            Self::Inline(array) => array.iter().any(|value| is_inline_profile(value, profile)),
        }
    }

    fn push(self, profile: &Profile) {
        match self {
            Self::Tables(tables) => {
                let mut table = toml_edit::Table::new();
                table["server"] = toml_edit::value(profile.server());
                table["username"] = toml_edit::value(profile.username());
                tables.push(table);
            }
            Self::Inline(array) => {
                let mut table = toml_edit::InlineTable::new();
                table.insert("server", profile.server().into());
                table.insert("username", profile.username().into());
                let indent = array
                    .iter()
                    .last()
                    .and_then(|last| last.decor().prefix()?.as_str()?.rsplit_once('\n'))
                    .map(|(_, indent)| format!("\n{indent}"));
                let Some(indent) = indent else {
                    array.push(table);
                    return;
                };
                // On a line of its own as the last one is, after what followed that one, such as
                // its comment
                let trailing = array.trailing().as_str().unwrap_or_default().to_owned();
                let mut value = toml_edit::Value::from(table);
                value
                    .decor_mut()
                    .set_prefix(format!("{}{indent}", trailing.trim_end()));
                array.set_trailing(
                    trailing
                        .rsplit_once('\n')
                        .map_or_else(String::new, |(_, end)| format!("\n{end}")),
                );
                array.push_formatted(value);
            }
        }
    }

    fn remove(&mut self, profile: &Profile) {
        match self {
            Self::Tables(tables) => tables.retain(|table| !is_profile(table, profile)),
            Self::Inline(array) => array.retain(|value| !is_inline_profile(value, profile)),
        }
    }

    fn is_empty(&self) -> bool {
        match self {
            Self::Tables(tables) => tables.is_empty(),
            Self::Inline(array) => array.is_empty(),
        }
    }
}

fn is_profile(table: &dyn toml_edit::TableLike, profile: &Profile) -> bool {
    let saved = |key| table.get(key).and_then(toml_edit::Item::as_str);
    saved("server")
        .zip(saved("username"))
        .is_some_and(|(server, username)| Profile::new(server, username) == *profile)
}

fn is_inline_profile(value: &toml_edit::Value, profile: &Profile) -> bool {
    value
        .as_inline_table()
        .is_some_and(|table| is_profile(table, profile))
}

/// How many bytes a cache limit of `gib` keeps, where `0` means no limit; `None`
/// for a negative number or one that is not.
pub fn cache_capacity_of(gib: f64) -> Option<u64> {
    (gib.is_finite() && gib >= 0.0).then(|| {
        if gib == 0.0 {
            u64::MAX
        } else {
            // Saturates for sizes no disk has
            (gib * f64::from(1u32 << 30)) as u64
        }
    })
}

fn usable_key(key: Option<String>) -> Option<String> {
    key.map(|key| key.trim().to_owned())
        .filter(|key| !key.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_store(name: &str) -> Store {
        let dir =
            std::env::temp_dir().join(format!("hiphile-config-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        Store::at(dir)
    }

    /// Reads without the process environment, so tests do not depend on it.
    fn read(store: &Store, env: &[(&str, &str)]) -> Settings {
        let env = env
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        store
            .read(config_rs::Environment::with_prefix(ENV_PREFIX).source(Some(env)))
            .unwrap()
    }

    fn load(store: &Store, env: &[(&str, &str)]) -> Option<Profile> {
        let settings = read(store, env);
        settings
            .server
            .zip(settings.username)
            .map(|(server, username)| Profile::new(server, username))
    }

    #[test]
    fn user_store_is_under_the_home_config_dir() {
        let store = Store::user().unwrap();
        assert!(
            store.dir().ends_with(".config/hiphile"),
            "{:?}",
            store.dir()
        );
        assert!(store.dir().starts_with(dirs::home_dir().unwrap()));
    }

    #[test]
    fn missing_file_means_no_profile() {
        assert_eq!(load(&temporary_store("missing"), &[]), None);
    }

    #[test]
    fn saved_profile_round_trips() {
        let store = temporary_store("round-trip");
        let profile = Profile::new("https://music.example.com", "alice");
        store.save_profile(&profile).unwrap();
        assert_eq!(load(&store, &[]), Some(profile));
    }

    #[test]
    fn environment_overrides_the_file() {
        let store = temporary_store("environment");
        store
            .save_profile(&Profile::new("https://music.example.com", "alice"))
            .unwrap();
        assert_eq!(
            load(&store, &[("HIPHILE_SERVER", "https://other.example.com")]),
            Some(Profile::new("https://other.example.com", "alice"))
        );
    }

    #[test]
    fn environment_alone_is_a_profile() {
        let store = temporary_store("environment-alone");
        assert_eq!(
            load(
                &store,
                &[
                    ("HIPHILE_SERVER", "https://music.example.com"),
                    ("HIPHILE_USERNAME", "bob"),
                ]
            ),
            Some(Profile::new("https://music.example.com", "bob"))
        );
    }

    #[test]
    fn saving_keeps_other_settings_and_comments() {
        let store = temporary_store("keep");
        std::fs::create_dir_all(store.dir()).unwrap();
        std::fs::write(
            store.config_path(),
            "# My settings\nserver = \"https://old.example.com\"\nvolume = 0.5\n",
        )
        .unwrap();
        store
            .save_profile(&Profile::new("https://music.example.com", "alice"))
            .unwrap();
        let text = std::fs::read_to_string(store.config_path()).unwrap();
        assert!(text.contains("# My settings"), "{text}");
        assert!(text.contains("volume = 0.5"), "{text}");
        assert!(
            text.contains("server = \"https://music.example.com\""),
            "{text}"
        );
        assert!(text.contains("username = \"alice\""), "{text}");
    }

    #[test]
    fn volume_round_trips_next_to_the_profile() {
        let store = temporary_store("volume");
        let profile = Profile::new("https://music.example.com", "alice");
        store.save_profile(&profile).unwrap();
        store.save_volume(0.123).unwrap();
        assert_eq!(read(&store, &[]).volume, Some(0.12));
        assert_eq!(load(&store, &[]), Some(profile));
        assert_eq!(read(&store, &[("HIPHILE_VOLUME", "0.5")]).volume, Some(0.5));
    }

    #[test]
    fn output_round_trips_and_clears() {
        let store = temporary_store("output");
        let profile = Profile::new("https://music.example.com", "alice");
        store.save_profile(&profile).unwrap();

        store.save_output_device(Some("coreaudio:device")).unwrap();
        store.save_sample_rate(SampleRate::Fixed(96_000)).unwrap();
        let settings = read(&store, &[]);
        assert_eq!(settings.output_device.as_deref(), Some("coreaudio:device"));
        assert_eq!(store.sample_rate().unwrap(), SampleRate::Fixed(96_000));

        store.save_output_device(None).unwrap();
        store.save_sample_rate(SampleRate::MatchSong).unwrap();
        let settings = read(&store, &[]);
        assert_eq!(settings.output_device, None);
        assert_eq!(settings.sample_rate, None);
        assert_eq!(load(&store, &[]), Some(profile));
    }

    #[test]
    fn a_song_multiple_round_trips() {
        let store = temporary_store("sample-rate-multiple");
        for multiple in SAMPLE_RATE_MULTIPLES {
            let rate = SampleRate::SongMultiple(multiple);
            store.save_sample_rate(rate).unwrap();
            assert_eq!(store.sample_rate().unwrap(), rate);
        }
        let text = std::fs::read_to_string(store.config_path()).unwrap();
        assert!(text.contains("sample_rate = \"x16\""), "{text}");
    }

    #[test]
    fn a_sample_rate_is_a_rate_or_an_offered_multiple() {
        let store = temporary_store("sample-rate-invalid");
        std::fs::create_dir_all(store.dir()).unwrap();
        let rate = |text: &str| {
            std::fs::write(store.config_path(), text).unwrap();
            store.sample_rate()
        };
        assert_eq!(rate("sample_rate = 0\n").unwrap(), SampleRate::MatchSong);
        assert_eq!(
            rate("sample_rate = 48000\n").unwrap(),
            SampleRate::Fixed(48_000)
        );
        for text in [
            "sample_rate = \"x3\"\n",
            "sample_rate = \"fast\"\n",
            "sample_rate = -1\n",
        ] {
            assert!(
                matches!(
                    rate(text),
                    Err(Error::InvalidSetting {
                        key: "sample_rate",
                        ..
                    })
                ),
                "{text}"
            );
        }
    }

    #[test]
    fn trim_silence_round_trips_and_is_off_unless_told_otherwise() {
        let store = temporary_store("trim-silence");
        assert!(!store.trim_silence().unwrap());

        store.save_trim_silence(true).unwrap();
        assert!(store.trim_silence().unwrap());

        store.save_trim_silence(false).unwrap();
        assert!(!store.trim_silence().unwrap());
        let text = std::fs::read_to_string(store.config_path()).unwrap();
        assert!(!text.contains("trim_silence"), "{text}");
    }

    #[test]
    fn skip_instrumentals_round_trips_and_is_off_unless_told_otherwise() {
        let store = temporary_store("skip-instrumentals");
        assert!(!store.skip_instrumentals().unwrap());

        store.save_skip_instrumentals(true).unwrap();
        assert!(store.skip_instrumentals().unwrap());

        store.save_skip_instrumentals(false).unwrap();
        assert!(!store.skip_instrumentals().unwrap());
        let text = std::fs::read_to_string(store.config_path()).unwrap();
        assert!(!text.contains("skip_instrumentals"), "{text}");
    }

    #[test]
    fn environment_overrides_the_saved_skip_instrumentals() {
        let store = temporary_store("skip-instrumentals-environment");
        let settings = read(&store, &[("HIPHILE_SKIP_INSTRUMENTALS", "true")]);
        assert_eq!(settings.skip_instrumentals, Some(true));
    }

    #[test]
    fn play_pause_fade_round_trips_and_is_off_unless_told_otherwise() {
        let store = temporary_store("play-pause-fade");
        assert!(!store.play_pause_fade().unwrap());

        store.save_play_pause_fade(true).unwrap();
        assert!(store.play_pause_fade().unwrap());

        store.save_play_pause_fade(false).unwrap();
        assert!(!store.play_pause_fade().unwrap());
        let text = std::fs::read_to_string(store.config_path()).unwrap();
        assert!(!text.contains("play_pause_fade"), "{text}");
    }

    #[test]
    fn crossfade_round_trips_and_is_off_unless_told_otherwise() {
        let store = temporary_store("crossfade");
        assert_eq!(store.crossfade_seconds().unwrap(), 0);

        store.save_crossfade_seconds(5).unwrap();
        assert_eq!(store.crossfade_seconds().unwrap(), 5);

        store.save_crossfade_seconds(0).unwrap();
        assert_eq!(store.crossfade_seconds().unwrap(), 0);
        let text = std::fs::read_to_string(store.config_path()).unwrap();
        assert!(!text.contains("crossfade_seconds"), "{text}");
    }

    #[test]
    fn a_crossfade_longer_than_the_longest_is_reported() {
        let store = temporary_store("long-crossfade");
        store
            .save_crossfade_seconds(LONGEST_CROSSFADE_SECONDS)
            .unwrap();
        assert!(store.crossfade_seconds().is_ok());
        store
            .save_crossfade_seconds(LONGEST_CROSSFADE_SECONDS + 1)
            .unwrap();
        assert!(matches!(
            store.crossfade_seconds(),
            Err(Error::InvalidSetting {
                key: "crossfade_seconds",
                ..
            })
        ));
    }

    fn custom_crossfeed() -> Crossfeed {
        Crossfeed {
            is_on: true,
            preset: CrossfeedPreset::Custom(CrossfeedFilter {
                cutoff_hz: 900,
                separation_db: 8.0,
            }),
        }
    }

    fn written(store: &Store) -> String {
        std::fs::read_to_string(store.config_path()).unwrap_or_default()
    }

    #[test]
    fn crossfeed_is_off_and_bs2b_unless_told_otherwise() {
        let store = temporary_store("crossfeed-default");
        assert_eq!(store.crossfeed().unwrap(), Crossfeed::default());
        assert!(!Crossfeed::default().is_on);
        assert_eq!(Crossfeed::default().preset, CrossfeedPreset::Bs2b);
    }

    #[test]
    fn crossfeed_round_trips_and_its_defaults_take_no_keys() {
        let store = temporary_store("crossfeed-round-trip");
        let on = Crossfeed {
            is_on: true,
            ..Crossfeed::default()
        };
        store
            .save_crossfeed_changes(Crossfeed::default(), on)
            .unwrap();
        assert_eq!(store.crossfeed().unwrap(), on);
        assert_eq!(written(&store).trim(), "crossfeed = true");

        store
            .save_crossfeed_changes(on, custom_crossfeed())
            .unwrap();
        assert_eq!(store.crossfeed().unwrap(), custom_crossfeed());
        let text = written(&store);
        assert!(text.contains("crossfeed_preset = \"custom\""), "{text}");
        assert!(text.contains("crossfeed_cutoff_hz = 900"), "{text}");
        assert!(text.contains("crossfeed_separation_db = 8.0"), "{text}");
    }

    #[test]
    fn crossfeed_presets_are_saved_by_name_without_what_custom_leaves_behind() {
        let store = temporary_store("crossfeed-presets");
        let mut saved = custom_crossfeed();
        store
            .save_crossfeed_changes(Crossfeed::default(), saved)
            .unwrap();
        for (preset, name) in [
            (CrossfeedPreset::ChuMoy, "chu-moy"),
            (CrossfeedPreset::JanMeier, "jan-meier"),
        ] {
            let crossfeed = Crossfeed {
                preset,
                ..custom_crossfeed()
            };
            store.save_crossfeed_changes(saved, crossfeed).unwrap();
            saved = crossfeed;
            assert_eq!(store.crossfeed().unwrap(), crossfeed);
            let text = written(&store);
            assert!(
                text.contains(&format!("crossfeed_preset = \"{name}\"")),
                "{text}"
            );
            for key in CROSSFEED_FILTER_KEYS {
                assert!(!text.contains(key), "{text}");
            }
        }
        let bs2b = Crossfeed {
            preset: CrossfeedPreset::Bs2b,
            ..saved
        };
        store.save_crossfeed_changes(saved, bs2b).unwrap();
        assert!(!written(&store).contains("crossfeed_preset"));
    }

    #[test]
    fn the_preset_stays_while_crossfeed_is_off() {
        let store = temporary_store("crossfeed-off");
        let jan_meier = Crossfeed {
            is_on: true,
            preset: CrossfeedPreset::JanMeier,
        };
        store
            .save_crossfeed_changes(Crossfeed::default(), jan_meier)
            .unwrap();
        let off = Crossfeed {
            is_on: false,
            ..jan_meier
        };
        store.save_crossfeed_changes(jan_meier, off).unwrap();
        assert_eq!(store.crossfeed().unwrap(), off);
        let text = written(&store);
        assert!(!text.contains("crossfeed ="), "{text}");
        assert!(text.contains("crossfeed_preset = \"jan-meier\""), "{text}");
    }

    #[test]
    fn only_what_changed_of_the_crossfeed_is_saved() {
        let store = temporary_store("crossfeed-changes");
        let from_environment = Crossfeed {
            is_on: false,
            ..custom_crossfeed()
        };
        store
            .save_crossfeed_changes(from_environment, custom_crossfeed())
            .unwrap();
        assert_eq!(written(&store).trim(), "crossfeed = true");

        let mut filter = custom_crossfeed().preset.filter();
        filter.cutoff_hz = 1000;
        let wider = Crossfeed {
            preset: CrossfeedPreset::Custom(filter),
            ..custom_crossfeed()
        };
        store
            .save_crossfeed_changes(custom_crossfeed(), wider)
            .unwrap();
        let text = written(&store);
        assert!(text.contains("crossfeed_preset = \"custom\""), "{text}");
        assert!(text.contains("crossfeed_cutoff_hz = 1000"), "{text}");
        assert!(!text.contains("crossfeed_separation_db"), "{text}");
    }

    #[test]
    fn the_environment_sets_a_saved_crossfeed_aside() {
        let store = temporary_store("crossfeed-environment");
        store
            .save_crossfeed_changes(Crossfeed::default(), custom_crossfeed())
            .unwrap();
        let settings = read(
            &store,
            &[
                ("HIPHILE_CROSSFEED", "false"),
                ("HIPHILE_CROSSFEED_SEPARATION_DB", "12.5"),
            ],
        );
        let crossfeed = store.parse_crossfeed(&settings).unwrap();
        assert!(!crossfeed.is_on);
        let filter = crossfeed.preset.filter();
        assert_eq!(filter.separation_db, 12.5);
        assert_eq!(filter.cutoff_hz, 900);

        let settings = read(&store, &[("HIPHILE_CROSSFEED_PRESET", "chu-moy")]);
        assert_eq!(
            store.parse_crossfeed(&settings).unwrap().preset,
            CrossfeedPreset::ChuMoy
        );
    }

    #[test]
    fn a_custom_separation_is_read_in_tenths() {
        let store = temporary_store("crossfeed-tenths");
        std::fs::create_dir_all(store.dir()).unwrap();
        for (written, read) in [(4.53, 4.5), (12.47, 12.5), (0.96, 1.0)] {
            std::fs::write(
                store.config_path(),
                format!("crossfeed_preset = \"custom\"\ncrossfeed_separation_db = {written}\n"),
            )
            .unwrap();
            let filter = store.crossfeed().unwrap().preset.filter();
            assert_eq!(filter.separation_db, read, "{written}");
        }
    }

    #[test]
    fn crossfeed_values_outside_their_ranges_are_named_and_ignored_for_presets() {
        let store = temporary_store("crossfeed-invalid");
        std::fs::create_dir_all(store.dir()).unwrap();
        let key = |text: &str| {
            std::fs::write(store.config_path(), text).unwrap();
            match store.crossfeed() {
                Err(Error::InvalidSetting { key, .. }) => Some(key),
                Err(error) => panic!("{error}"),
                Ok(_) => None,
            }
        };
        for (line, expected) in [
            ("crossfeed_cutoff_hz = 299", "crossfeed_cutoff_hz"),
            ("crossfeed_cutoff_hz = 2001", "crossfeed_cutoff_hz"),
            ("crossfeed_separation_db = 0.5", "crossfeed_separation_db"),
            ("crossfeed_separation_db = 15.5", "crossfeed_separation_db"),
        ] {
            assert_eq!(
                key(&format!("crossfeed_preset = \"custom\"\n{line}\n")),
                Some(expected)
            );
            assert_eq!(key(&format!("crossfeed_preset = \"bs2b\"\n{line}\n")), None);
        }
        assert_eq!(
            key("crossfeed_preset = \"strong\"\n"),
            Some("crossfeed_preset")
        );
        assert_eq!(
            key(
                "crossfeed_preset = \"custom\"\ncrossfeed_cutoff_hz = 2000\n\
                 crossfeed_separation_db = 1\n"
            ),
            None
        );
    }

    #[test]
    fn balance_round_trips_and_is_centered_unless_told_otherwise() {
        let store = temporary_store("balance");
        assert_eq!(store.balance_db().unwrap(), 0.0);
        store.save_balance_db(-1.5).unwrap();
        assert_eq!(store.balance_db().unwrap(), -1.5);
        store.save_balance_db(0.0).unwrap();
        assert_eq!(store.balance_db().unwrap(), 0.0);
        assert!(!written(&store).contains("balance_db"));
    }

    #[test]
    fn a_balance_beyond_12_db_is_reported() {
        let store = temporary_store("balance-invalid");
        store.save_balance_db(12.0).unwrap();
        assert!(store.balance_db().is_ok());
        for db in [12.5, -13.0] {
            store.save_balance_db(db).unwrap();
            assert!(matches!(
                store.balance_db(),
                Err(Error::InvalidSetting {
                    key: "balance_db",
                    ..
                })
            ));
        }
    }

    #[test]
    fn exclusive_mode_round_trips_and_is_off_unless_told_otherwise() {
        let store = temporary_store("exclusive-mode");
        assert!(!store.exclusive_mode().unwrap());

        store.save_exclusive_mode(true).unwrap();
        assert!(store.exclusive_mode().unwrap());

        store.save_exclusive_mode(false).unwrap();
        assert!(!store.exclusive_mode().unwrap());
        let text = std::fs::read_to_string(store.config_path()).unwrap();
        assert!(!text.contains("exclusive_mode"), "{text}");
    }

    #[test]
    fn resampling_quality_round_trips_and_is_medium_unless_told_otherwise() {
        let store = temporary_store("resampling");
        assert_eq!(
            store.resampling_quality().unwrap(),
            ResamplingQuality::Medium
        );

        let custom = ResamplingQuality::Custom {
            fft_size: 2048,
            window: ResamplingWindow::Hann2,
        };
        for quality in [
            custom,
            ResamplingQuality::High,
            ResamplingQuality::Max,
            ResamplingQuality::Medium,
        ] {
            store.save_resampling_quality(quality).unwrap();
            assert_eq!(store.resampling_quality().unwrap(), quality);
        }

        let text = std::fs::read_to_string(store.config_path()).unwrap();
        assert!(!text.contains("resampling_fft_size"), "{text}");
        assert!(!text.contains("resampling_window"), "{text}");
    }

    #[test]
    fn r8brain_is_saved_by_name_without_what_custom_leaves_behind() {
        let store = temporary_store("resampling-r8brain");
        store
            .save_resampling_quality(ResamplingQuality::Custom {
                fft_size: 2048,
                window: ResamplingWindow::Hann2,
            })
            .unwrap();
        let quality = ResamplingQuality::R8brain(R8brainParameters::default());
        store.save_resampling_quality(quality).unwrap();

        assert_eq!(store.resampling_quality().unwrap(), quality);
        let text = std::fs::read_to_string(store.config_path()).unwrap();
        assert!(text.contains("resampling = \"r8brain\""), "{text}");
        assert!(!text.contains("resampling_fft_size"), "{text}");
        assert!(!text.contains("resampling_window"), "{text}");
    }

    fn unusual_r8brain_parameters() -> R8brainParameters {
        R8brainParameters {
            quality: R8brainQuality::Custom(R8brainFilter {
                transition_band: 3.5,
                attenuation: 136.45,
                phase: R8brainPhase::Minimum,
                whole_stepping: false,
                filter_fractions: Some(2048),
            }),
            performance: R8brainPerformance {
                max_input_frames: 4096,
                extended_fft: true,
                filter_cache_size: 8,
                filter_bank_cache_size: 3,
            },
        }
    }

    #[test]
    fn r8brain_parameters_round_trip_and_defaults_take_no_keys() {
        let store = temporary_store("r8brain-parameters");
        let parameters = unusual_r8brain_parameters();
        store
            .save_r8brain_changes(R8brainParameters::default(), parameters)
            .unwrap();
        store
            .save_resampling_quality(ResamplingQuality::R8brain(parameters))
            .unwrap();
        assert_eq!(
            store.resampling_quality().unwrap(),
            ResamplingQuality::R8brain(parameters)
        );

        store
            .save_r8brain_changes(parameters, R8brainParameters::default())
            .unwrap();
        let text = std::fs::read_to_string(store.config_path()).unwrap();
        assert!(!text.contains("r8brain_"), "{text}");
    }

    #[test]
    fn r8brain_presets_are_saved_by_name_without_what_custom_leaves_behind() {
        let store = temporary_store("r8brain-presets");
        let mut saved = unusual_r8brain_parameters();
        store
            .save_r8brain_changes(R8brainParameters::default(), saved)
            .unwrap();
        for quality in [
            R8brainQuality::Low,
            R8brainQuality::Medium,
            R8brainQuality::Max,
        ] {
            let parameters = R8brainParameters {
                quality,
                ..unusual_r8brain_parameters()
            };
            store.save_r8brain_changes(saved, parameters).unwrap();
            saved = parameters;
            assert_eq!(store.r8brain_parameters().unwrap(), parameters);
            let text = std::fs::read_to_string(store.config_path()).unwrap();
            assert!(
                text.contains(&format!("r8brain_quality = \"{}\"", quality.name())),
                "{text}"
            );
            for key in R8BRAIN_FILTER_KEYS {
                assert!(!text.contains(key), "{text}");
            }
            assert!(text.contains("r8brain_max_input_frames"), "{text}");
        }
    }

    #[test]
    fn only_what_changed_of_the_r8brain_parameters_is_saved() {
        let store = temporary_store("r8brain-changes");
        let from_environment = R8brainParameters {
            quality: R8brainQuality::Custom(R8brainFilter {
                attenuation: 100.0,
                ..R8brainQuality::High.filter()
            }),
            ..R8brainParameters::default()
        };
        let mut larger_block = from_environment;
        larger_block.performance.max_input_frames = 4096;
        store
            .save_r8brain_changes(from_environment, larger_block)
            .unwrap();
        let text = std::fs::read_to_string(store.config_path()).unwrap();
        assert!(text.contains("r8brain_max_input_frames = 4096"), "{text}");
        for key in ["r8brain_quality", "r8brain_attenuation"] {
            assert!(!text.contains(key), "{text}");
        }

        let mut filter = larger_block.quality.filter();
        filter.transition_band = 3.0;
        let wider = R8brainParameters {
            quality: R8brainQuality::Custom(filter),
            ..larger_block
        };
        store.save_r8brain_changes(larger_block, wider).unwrap();
        let text = std::fs::read_to_string(store.config_path()).unwrap();
        assert!(text.contains("r8brain_quality = \"custom\""), "{text}");
        assert!(text.contains("r8brain_transition_band = 3.0"), "{text}");
        assert!(!text.contains("r8brain_attenuation"), "{text}");
    }

    #[test]
    fn leaving_a_preset_for_custom_drops_what_the_preset_ignored() {
        let store = temporary_store("r8brain-leftover");
        std::fs::create_dir_all(store.dir()).unwrap();
        std::fs::write(store.config_path(), "r8brain_attenuation = 120\n").unwrap();
        let before = store.r8brain_parameters().unwrap();
        let after = R8brainParameters {
            quality: R8brainQuality::Custom(before.quality.filter()),
            ..before
        };
        store.save_r8brain_changes(before, after).unwrap();
        assert_eq!(store.r8brain_parameters().unwrap(), after);
    }

    #[test]
    fn r8brain_presets_rise_in_every_way() {
        let filters = [
            R8brainQuality::Low,
            R8brainQuality::Medium,
            R8brainQuality::High,
            R8brainQuality::Max,
        ]
        .map(R8brainQuality::filter);
        for pair in filters.windows(2) {
            assert!(pair[0].transition_band > pair[1].transition_band);
            assert!(pair[0].attenuation <= pair[1].attenuation);
        }
        for filter in filters {
            assert!(R8BRAIN_TRANSITION_BANDS.contains(&filter.transition_band));
            assert!(R8BRAIN_ATTENUATIONS.contains(&filter.attenuation));
        }
    }

    #[test]
    fn r8brain_custom_fills_in_from_high_and_values_are_read_only_for_it() {
        let store = temporary_store("r8brain-custom");
        std::fs::create_dir_all(store.dir()).unwrap();
        std::fs::write(
            store.config_path(),
            "r8brain_quality = \"custom\"\nr8brain_attenuation = 100\n",
        )
        .unwrap();
        let filter = store.r8brain_parameters().unwrap().quality.filter();
        assert_eq!(filter.attenuation, 100.0);
        assert_eq!(filter.transition_band, 2.0);

        std::fs::write(
            store.config_path(),
            "r8brain_quality = \"max\"\nr8brain_attenuation = 100\n",
        )
        .unwrap();
        let parameters = store.r8brain_parameters().unwrap();
        assert_eq!(parameters.quality, R8brainQuality::Max);
        assert_eq!(parameters.quality.filter().attenuation, 218.0);
    }

    #[test]
    fn r8brain_parameters_stay_while_another_quality_is_in_use() {
        let store = temporary_store("r8brain-kept");
        let parameters = unusual_r8brain_parameters();
        store
            .save_r8brain_changes(R8brainParameters::default(), parameters)
            .unwrap();
        store
            .save_resampling_quality(ResamplingQuality::R8brain(parameters))
            .unwrap();
        store
            .save_resampling_quality(ResamplingQuality::Max)
            .unwrap();

        assert_eq!(store.resampling_quality().unwrap(), ResamplingQuality::Max);
        assert_eq!(store.r8brain_parameters().unwrap(), parameters);
    }

    #[test]
    fn r8brain_parameters_default_to_what_the_original_has_for_24_bits() {
        let store = temporary_store("r8brain-default");
        let parameters = store.r8brain_parameters().unwrap();
        assert_eq!(parameters, R8brainParameters::default());
        assert_eq!(parameters.quality, R8brainQuality::High);
        let filter = parameters.quality.filter();
        assert_eq!(filter.transition_band, 2.0);
        assert_eq!(filter.attenuation, 180.15);
        assert_eq!(filter.filter_fractions, None);
    }

    #[test]
    fn whole_numbers_in_the_file_count_for_the_r8brain_numbers() {
        let store = temporary_store("r8brain-whole-numbers");
        std::fs::create_dir_all(store.dir()).unwrap();
        std::fs::write(
            store.config_path(),
            "r8brain_quality = \"custom\"\nr8brain_transition_band = 3\nr8brain_attenuation = 100\n",
        )
        .unwrap();
        let filter = store.r8brain_parameters().unwrap().quality.filter();
        assert_eq!(filter.transition_band, 3.0);
        assert_eq!(filter.attenuation, 100.0);
    }

    #[test]
    fn the_environment_sets_saved_r8brain_parameters_aside() {
        let store = temporary_store("r8brain-environment");
        store
            .save_r8brain_changes(R8brainParameters::default(), unusual_r8brain_parameters())
            .unwrap();
        let settings = read(
            &store,
            &[
                ("HIPHILE_R8BRAIN_ATTENUATION", "100.5"),
                ("HIPHILE_R8BRAIN_PHASE", "linear"),
                ("HIPHILE_R8BRAIN_EXTENDED_FFT", "false"),
                ("HIPHILE_R8BRAIN_FILTER_FRACTIONS", "64"),
            ],
        );
        let parameters = store.parse_r8brain_parameters(&settings).unwrap();
        let filter = parameters.quality.filter();
        assert_eq!(filter.attenuation, 100.5);
        assert_eq!(filter.phase, R8brainPhase::Linear);
        assert_eq!(filter.filter_fractions, Some(64));
        assert_eq!(filter.transition_band, 3.5);
        assert!(!parameters.performance.extended_fft);

        let settings = read(&store, &[("HIPHILE_R8BRAIN_QUALITY", "low")]);
        assert_eq!(
            store.parse_r8brain_parameters(&settings).unwrap().quality,
            R8brainQuality::Low
        );
    }

    #[test]
    fn r8brain_values_outside_their_ranges_are_named_and_ignored_for_other_qualities() {
        let store = temporary_store("r8brain-invalid");
        std::fs::create_dir_all(store.dir()).unwrap();
        let key = |text: &str| {
            std::fs::write(store.config_path(), text).unwrap();
            match store.resampling_quality() {
                Err(Error::InvalidSetting { key, .. }) => Some(key),
                Err(error) => panic!("{error}"),
                Ok(_) => None,
            }
        };
        for (line, expected) in [
            ("r8brain_transition_band = 0.4", "r8brain_transition_band"),
            ("r8brain_transition_band = 45.5", "r8brain_transition_band"),
            ("r8brain_attenuation = 48.0", "r8brain_attenuation"),
            ("r8brain_attenuation = 219.0", "r8brain_attenuation"),
            ("r8brain_phase = \"mixed\"", "r8brain_phase"),
            ("r8brain_max_input_frames = 0", "r8brain_max_input_frames"),
            (
                "r8brain_max_input_frames = 1048577",
                "r8brain_max_input_frames",
            ),
            ("r8brain_filter_fractions = 4", "r8brain_filter_fractions"),
            (
                "r8brain_filter_fractions = 65537",
                "r8brain_filter_fractions",
            ),
            ("r8brain_filter_cache_size = 1", "r8brain_filter_cache_size"),
            (
                "r8brain_filter_bank_cache_size = 4097",
                "r8brain_filter_bank_cache_size",
            ),
        ] {
            assert_eq!(
                key(&format!(
                    "resampling = \"r8brain\"\nr8brain_quality = \"custom\"\n{line}\n"
                )),
                Some(expected)
            );
            assert_eq!(key(&format!("resampling = \"max\"\n{line}\n")), None);
        }
        assert_eq!(
            key("resampling = \"r8brain\"\nr8brain_quality = \"best\"\n"),
            Some("r8brain_quality")
        );
        assert_eq!(
            key("resampling = \"r8brain\"\nr8brain_attenuation = 1\n"),
            None
        );
        assert_eq!(
            key("resampling = \"r8brain\"\nr8brain_quality = \"custom\"\n\
                 r8brain_attenuation = 218\nr8brain_transition_band = 0.5\n"),
            None
        );
    }

    fn unusual_soxr_parameters() -> SoxrParameters {
        SoxrParameters {
            quality: SoxrQuality::Custom(SoxrFilter {
                precision: 24.5,
                passband_end: 0.95,
                phase: SoxrPhase::Intermediate,
                double_precision: true,
            }),
            performance: SoxrPerformance {
                coef_table_kb: 1,
                coef_interpolation: SoxrInterpolation::Cubic,
                min_dft_size: 4096,
                large_dft_size: 1_048_576,
            },
        }
    }

    #[test]
    fn soxr_parameters_round_trip_by_name_and_defaults_take_no_keys() {
        let store = temporary_store("soxr-parameters");
        store
            .save_resampling_quality(ResamplingQuality::Custom {
                fft_size: 2048,
                window: ResamplingWindow::Hann2,
            })
            .unwrap();
        let parameters = unusual_soxr_parameters();
        store
            .save_soxr_changes(SoxrParameters::default(), parameters)
            .unwrap();
        store
            .save_resampling_quality(ResamplingQuality::Soxr(parameters))
            .unwrap();
        assert_eq!(
            store.resampling_quality().unwrap(),
            ResamplingQuality::Soxr(parameters)
        );
        let text = std::fs::read_to_string(store.config_path()).unwrap();
        assert!(text.contains("resampling = \"soxr\""), "{text}");
        assert!(!text.contains("resampling_fft_size"), "{text}");

        store
            .save_soxr_changes(parameters, SoxrParameters::default())
            .unwrap();
        let text = std::fs::read_to_string(store.config_path()).unwrap();
        assert!(!text.contains("soxr_"), "{text}");
    }

    #[test]
    fn soxr_presets_are_saved_by_name_without_what_custom_leaves_behind() {
        let store = temporary_store("soxr-presets");
        let mut saved = unusual_soxr_parameters();
        store
            .save_soxr_changes(SoxrParameters::default(), saved)
            .unwrap();
        for quality in [SoxrQuality::Low, SoxrQuality::Medium, SoxrQuality::High] {
            let parameters = SoxrParameters {
                quality,
                ..unusual_soxr_parameters()
            };
            store.save_soxr_changes(saved, parameters).unwrap();
            saved = parameters;
            assert_eq!(store.soxr_parameters().unwrap(), parameters);
            let text = std::fs::read_to_string(store.config_path()).unwrap();
            assert!(
                text.contains(&format!("soxr_quality = \"{}\"", quality.name())),
                "{text}"
            );
            for key in SOXR_FILTER_KEYS {
                assert!(!text.contains(key), "{text}");
            }
            assert!(text.contains("soxr_coef_interpolation"), "{text}");
        }
    }

    #[test]
    fn only_what_changed_of_the_soxr_parameters_is_saved() {
        let store = temporary_store("soxr-changes");
        let from_environment = SoxrParameters {
            quality: SoxrQuality::Custom(SoxrFilter {
                precision: 32.0,
                ..SoxrQuality::default().filter()
            }),
            ..SoxrParameters::default()
        };
        let mut larger_table = from_environment;
        larger_table.performance.coef_table_kb = 1600;
        store
            .save_soxr_changes(from_environment, larger_table)
            .unwrap();
        let text = std::fs::read_to_string(store.config_path()).unwrap();
        assert!(text.contains("soxr_coef_table_kb = 1600"), "{text}");
        for key in ["soxr_quality", "soxr_precision"] {
            assert!(!text.contains(key), "{text}");
        }

        let mut filter = larger_table.quality.filter();
        filter.phase = SoxrPhase::Minimum;
        let minimum = SoxrParameters {
            quality: SoxrQuality::Custom(filter),
            ..larger_table
        };
        store.save_soxr_changes(larger_table, minimum).unwrap();
        let text = std::fs::read_to_string(store.config_path()).unwrap();
        assert!(text.contains("soxr_quality = \"custom\""), "{text}");
        assert!(text.contains("soxr_phase = \"minimum\""), "{text}");
        assert!(!text.contains("soxr_precision"), "{text}");
    }

    #[test]
    fn soxr_defaults_to_very_high_and_custom_fills_in_from_it() {
        let store = temporary_store("soxr-custom");
        let parameters = store.soxr_parameters().unwrap();
        assert_eq!(parameters, SoxrParameters::default());
        assert_eq!(parameters.quality, SoxrQuality::VeryHigh);

        std::fs::create_dir_all(store.dir()).unwrap();
        std::fs::write(
            store.config_path(),
            "soxr_quality = \"custom\"\nsoxr_precision = 24\n",
        )
        .unwrap();
        let filter = store.soxr_parameters().unwrap().quality.filter();
        assert_eq!(filter.precision, 24.0);
        assert_eq!(
            filter.passband_end,
            SoxrQuality::VeryHigh.filter().passband_end
        );

        std::fs::write(
            store.config_path(),
            "soxr_quality = \"low\"\nsoxr_precision = 24\n",
        )
        .unwrap();
        let parameters = store.soxr_parameters().unwrap();
        assert_eq!(parameters.quality, SoxrQuality::Low);
        assert_eq!(parameters.quality.filter().precision, 16.0);
    }

    #[test]
    fn soxr_parameters_stay_while_another_quality_is_in_use() {
        let store = temporary_store("soxr-kept");
        let parameters = unusual_soxr_parameters();
        store
            .save_soxr_changes(SoxrParameters::default(), parameters)
            .unwrap();
        store
            .save_resampling_quality(ResamplingQuality::Soxr(parameters))
            .unwrap();
        store
            .save_resampling_quality(ResamplingQuality::R8brain(R8brainParameters::default()))
            .unwrap();

        assert_eq!(
            store.resampling_quality().unwrap(),
            ResamplingQuality::R8brain(R8brainParameters::default())
        );
        assert_eq!(store.soxr_parameters().unwrap(), parameters);
    }

    #[test]
    fn the_environment_sets_saved_soxr_parameters_aside() {
        let store = temporary_store("soxr-environment");
        store
            .save_soxr_changes(SoxrParameters::default(), unusual_soxr_parameters())
            .unwrap();
        let settings = read(
            &store,
            &[
                ("HIPHILE_SOXR_PRECISION", "20"),
                ("HIPHILE_SOXR_PHASE", "linear"),
                ("HIPHILE_SOXR_COEF_INTERPOLATION", "auto"),
                ("HIPHILE_SOXR_MIN_DFT_SIZE", "256"),
            ],
        );
        let parameters = store.parse_soxr_parameters(&settings).unwrap();
        let filter = parameters.quality.filter();
        assert_eq!(filter.precision, 20.0);
        assert_eq!(filter.phase, SoxrPhase::Linear);
        assert_eq!(filter.passband_end, 0.95);
        assert_eq!(
            parameters.performance.coef_interpolation,
            SoxrInterpolation::Auto
        );
        assert_eq!(parameters.performance.min_dft_size, 256);

        let settings = read(&store, &[("HIPHILE_SOXR_QUALITY", "high")]);
        assert_eq!(
            store.parse_soxr_parameters(&settings).unwrap().quality,
            SoxrQuality::High
        );
    }

    #[test]
    fn soxr_values_outside_their_ranges_are_named_and_ignored_for_other_qualities() {
        let store = temporary_store("soxr-invalid");
        std::fs::create_dir_all(store.dir()).unwrap();
        let key = |text: &str| {
            std::fs::write(store.config_path(), text).unwrap();
            match store.resampling_quality() {
                Err(Error::InvalidSetting { key, .. }) => Some(key),
                Err(error) => panic!("{error}"),
                Ok(_) => None,
            }
        };
        for (line, expected) in [
            ("soxr_precision = 14.9", "soxr_precision"),
            ("soxr_precision = 33.5", "soxr_precision"),
            ("soxr_passband_end = 0.49", "soxr_passband_end"),
            ("soxr_passband_end = 0.999", "soxr_passband_end"),
            ("soxr_passband_end = 95", "soxr_passband_end"),
            ("soxr_phase = \"maximum\"", "soxr_phase"),
            ("soxr_coef_table_kb = 0", "soxr_coef_table_kb"),
            ("soxr_coef_table_kb = 65537", "soxr_coef_table_kb"),
            (
                "soxr_coef_interpolation = \"low\"",
                "soxr_coef_interpolation",
            ),
            ("soxr_min_dft_size = 128", "soxr_min_dft_size"),
            ("soxr_min_dft_size = 1000", "soxr_min_dft_size"),
            ("soxr_min_dft_size = 65536", "soxr_min_dft_size"),
            ("soxr_large_dft_size = 2097152", "soxr_large_dft_size"),
        ] {
            assert_eq!(
                key(&format!(
                    "resampling = \"soxr\"\nsoxr_quality = \"custom\"\n{line}\n"
                )),
                Some(expected)
            );
            assert_eq!(key(&format!("resampling = \"max\"\n{line}\n")), None);
        }
        assert_eq!(
            key("resampling = \"soxr\"\nsoxr_quality = \"best\"\n"),
            Some("soxr_quality")
        );
        assert_eq!(key("resampling = \"soxr\"\nsoxr_precision = 1\n"), None);
        assert_eq!(
            key("resampling = \"soxr\"\nsoxr_quality = \"custom\"\n\
                 soxr_precision = 33\nsoxr_passband_end = 0.998\n\
                 soxr_min_dft_size = 32768\nsoxr_large_dft_size = 256\n"),
            None
        );
    }

    #[test]
    fn soxr_presets_are_in_the_ranges_and_rise_in_precision() {
        let filters = [
            SoxrQuality::Low,
            SoxrQuality::Medium,
            SoxrQuality::High,
            SoxrQuality::VeryHigh,
        ]
        .map(SoxrQuality::filter);
        for pair in filters.windows(2) {
            assert!(pair[0].precision <= pair[1].precision);
        }
        for filter in filters {
            assert!(SOXR_PRECISIONS.contains(&filter.precision));
            assert!(SOXR_PASSBAND_ENDS.contains(&filter.passband_end));
        }
        let default = SoxrPerformance::default();
        assert!(SOXR_COEF_TABLE_KBS.contains(&default.coef_table_kb));
        assert!(SOXR_MIN_DFT_SIZES.contains(&default.min_dft_size));
        assert!(SOXR_LARGE_DFT_SIZES.contains(&default.large_dft_size));
    }

    #[test]
    fn resampling_presets_keep_their_sizes_and_custom_fills_in_from_medium() {
        let store = temporary_store("resampling-presets");
        let sizes = [
            ResamplingQuality::Medium,
            ResamplingQuality::High,
            ResamplingQuality::Max,
        ]
        .map(ResamplingQuality::fft_size);
        assert_eq!(sizes, [256, 1024, 4096]);

        std::fs::create_dir_all(store.dir()).unwrap();
        std::fs::write(store.config_path(), "resampling = \"custom\"\n").unwrap();
        assert_eq!(
            store.resampling_quality().unwrap(),
            ResamplingQuality::Custom {
                fft_size: 256,
                window: ResamplingWindow::BlackmanHarris2,
            }
        );
    }

    #[test]
    fn dsd_is_off_and_high_unless_told_otherwise() {
        let store = temporary_store("dsd-default");
        assert_eq!(store.dsd_output().unwrap(), DsdOutput::default());
        assert_eq!(DsdOutput::default().quality, DsdQuality::High);
    }

    #[test]
    fn dsd_output_round_trips() {
        let store = temporary_store("dsd-round-trip");
        let custom = |filter, limiter_ceiling_db| {
            DsdQuality::Custom(DsdParameters {
                filter,
                order: 9,
                noise_gain: 1.35,
                wide_noise_band: true,
                optimized_zeros: false,
                dither_db: Some(-100.0),
                input_level_db: -3.0,
                limiter_ceiling_db,
            })
        };
        let sinc = DsdFilter::Sinc {
            length: 2048,
            window: ResamplingWindow::Hann2,
            cutoff: Some(0.75),
        };
        let fft = DsdFilter::Fft {
            size: 8192,
            window: ResamplingWindow::Blackman,
        };
        for quality in [
            DsdQuality::Low,
            DsdQuality::Medium,
            DsdQuality::High,
            DsdQuality::Max,
            custom(sinc, Some(0.0)),
            custom(fft, None),
        ] {
            let output = DsdOutput {
                is_on: true,
                device: Some("coreaudio:dac".to_owned()),
                rate: Some(DsdRate::Dsd128),
                matches_song_family: true,
                quality,
                fades_in: false,
            };
            let before = store.dsd_output().unwrap();
            store.save_dsd_changes(&before, &output).unwrap();
            assert_eq!(store.dsd_output().unwrap(), output);
        }
    }

    #[test]
    fn a_dsd_preset_leaves_no_custom_values_in_the_file() {
        let store = temporary_store("dsd-preset");
        let custom = DsdOutput {
            quality: DsdQuality::Custom(DsdQuality::High.parameters()),
            ..DsdOutput::default()
        };
        store
            .save_dsd_changes(&DsdOutput::default(), &custom)
            .unwrap();
        store
            .save_dsd_changes(&custom, &DsdOutput::default())
            .unwrap();
        let text = std::fs::read_to_string(store.config_path()).unwrap();
        for key in DSD_PARAMETER_KEYS {
            assert!(!text.contains(key), "{text}");
        }
    }

    #[test]
    fn only_what_changed_of_the_dsd_output_is_saved() {
        let store = temporary_store("dsd-changes");
        let from_environment = DsdOutput {
            is_on: true,
            device: Some("coreaudio:dac".to_owned()),
            rate: Some(DsdRate::Dsd64),
            ..DsdOutput::default()
        };
        let custom = DsdOutput {
            quality: DsdQuality::Custom(DsdQuality::High.parameters()),
            ..from_environment.clone()
        };
        store.save_dsd_changes(&from_environment, &custom).unwrap();
        let text = std::fs::read_to_string(store.config_path()).unwrap();
        for key in ["output_format", "dop_device", "dsd_rate"] {
            assert!(!text.contains(key), "{text}");
        }
        assert!(text.contains("dsd_quality = \"custom\""), "{text}");
        assert!(text.contains("dsd_order"), "{text}");

        std::fs::write(store.config_path(), "").unwrap();
        let mut parameters = DsdQuality::High.parameters();
        parameters.order = 9;
        let changed = DsdOutput {
            quality: DsdQuality::Custom(parameters),
            ..from_environment
        };
        store.save_dsd_changes(&custom, &changed).unwrap();
        let text = std::fs::read_to_string(store.config_path()).unwrap();
        assert!(text.contains("dsd_order = 9"), "{text}");
        assert!(!text.contains("dsd_noise_gain"), "{text}");
    }

    #[test]
    fn dsd_fades_in_unless_told_otherwise() {
        let store = temporary_store("dsd-fade-in");
        assert!(store.dsd_output().unwrap().fades_in);
        let off = DsdOutput {
            fades_in: false,
            ..DsdOutput::default()
        };
        store.save_dsd_changes(&DsdOutput::default(), &off).unwrap();
        assert!(!store.dsd_output().unwrap().fades_in);
        store.save_dsd_changes(&off, &DsdOutput::default()).unwrap();
        let text = std::fs::read_to_string(store.config_path()).unwrap();
        assert!(!text.contains("dsd_fade_in"), "{text}");
    }

    #[test]
    fn a_dsd_cutoff_is_saved_as_written() {
        let store = temporary_store("dsd-cutoff");
        let mut parameters = DsdQuality::High.parameters();
        parameters.filter = DsdFilter::Sinc {
            length: 1024,
            window: ResamplingWindow::BlackmanHarris2,
            cutoff: Some(0.85),
        };
        let output = DsdOutput {
            quality: DsdQuality::Custom(parameters),
            ..DsdOutput::default()
        };
        store
            .save_dsd_changes(&DsdOutput::default(), &output)
            .unwrap();
        let text = std::fs::read_to_string(store.config_path()).unwrap();
        assert!(text.contains("dsd_cutoff = 0.85\n"), "{text}");
        assert_eq!(store.dsd_output().unwrap(), output);
    }

    #[test]
    fn dsd_presets_differ_in_their_order_and_filter_length() {
        let sinc_length = |quality: DsdQuality| match quality.parameters().filter {
            DsdFilter::Sinc { length, .. } => length,
            DsdFilter::Fft { .. } => 0,
        };
        assert_eq!(DsdQuality::Low.parameters().order, 5);
        assert_eq!(sinc_length(DsdQuality::Low), 256);
        assert_eq!(sinc_length(DsdQuality::Medium), 512);
        assert_eq!(sinc_length(DsdQuality::High), 1024);
        assert_eq!(sinc_length(DsdQuality::Max), 2048);
    }

    #[test]
    fn the_environment_sets_a_saved_dsd_output_aside() {
        let store = temporary_store("dsd-environment");
        std::fs::create_dir_all(store.dir()).unwrap();
        std::fs::write(store.config_path(), "output_format = \"pcm\"\n").unwrap();
        let settings = read(
            &store,
            &[
                ("HIPHILE_OUTPUT_FORMAT", "dsd"),
                ("HIPHILE_DSD_RATE", "64"),
                ("HIPHILE_DSD_QUALITY", "custom"),
                ("HIPHILE_DSD_NOISE_GAIN", "1.3"),
                ("HIPHILE_DSD_LIMITER_CEILING_DB", "off"),
            ],
        );
        let output = store.parse_dsd_output(&settings).unwrap();
        assert!(output.is_on);
        assert_eq!(output.rate, Some(DsdRate::Dsd64));
        let parameters = output.quality.parameters();
        assert!((parameters.noise_gain - 1.3).abs() < 1e-9);
        assert_eq!(parameters.limiter_ceiling_db, None);
    }

    #[test]
    fn unknown_dsd_values_are_named() {
        let store = temporary_store("dsd-invalid");
        std::fs::create_dir_all(store.dir()).unwrap();
        let key = |text: &str| {
            std::fs::write(store.config_path(), text).unwrap();
            match store.dsd_output() {
                Err(Error::InvalidSetting { key, .. }) => Some(key),
                Err(error) => panic!("{error}"),
                Ok(_) => None,
            }
        };
        assert_eq!(key("output_format = \"sacd\"\n"), Some("output_format"));
        assert_eq!(key("dsd_rate = 512\n"), Some("dsd_rate"));
        assert_eq!(key("dsd_base_rate = 48000\n"), Some("dsd_base_rate"));
        assert_eq!(key("dsd_base_rate = 44100\n"), None);
        assert_eq!(key("dsd_quality = \"best\"\n"), Some("dsd_quality"));
        assert_eq!(key("dsd_quality = \"balanced\"\n"), Some("dsd_quality"));
        let custom = |line: &str| format!("dsd_quality = \"custom\"\n{line}\n");
        for (line, expected) in [
            ("dsd_order = 4", "dsd_order"),
            ("dsd_noise_gain = 1.6", "dsd_noise_gain"),
            ("dsd_noise_band = 30000", "dsd_noise_band"),
            ("dsd_filter = \"iir\"", "dsd_filter"),
            ("dsd_sinc_length = 300", "dsd_sinc_length"),
            ("dsd_cutoff = 1.5", "dsd_cutoff"),
            ("dsd_window = \"flat\"", "dsd_window"),
            ("dsd_dither_db = 0", "dsd_dither_db"),
            ("dsd_input_level_db = 12", "dsd_input_level_db"),
            ("dsd_limiter_ceiling_db = 9", "dsd_limiter_ceiling_db"),
            ("dsd_limiter_ceiling_db = \"on\"", "dsd_limiter_ceiling_db"),
            ("dsd_filter = \"fft\"\ndsd_fft_size = 300", "dsd_fft_size"),
        ] {
            assert_eq!(key(&custom(line)), Some(expected), "{line}");
        }
        // Without custom they are not read
        assert_eq!(key("dsd_order = 4\n"), None);
    }

    #[test]
    fn the_environment_sets_a_saved_resampling_quality_aside() {
        let store = temporary_store("resampling-environment");
        store
            .save_resampling_quality(ResamplingQuality::Max)
            .unwrap();
        let settings = read(&store, &[("HIPHILE_RESAMPLING", "high")]);
        assert_eq!(
            store.parse_resampling_quality(&settings).unwrap(),
            ResamplingQuality::High
        );
    }

    #[test]
    fn unknown_resampling_values_are_named_and_ignored_without_custom() {
        let store = temporary_store("resampling-invalid");
        std::fs::create_dir_all(store.dir()).unwrap();
        let key = |text: &str| {
            std::fs::write(store.config_path(), text).unwrap();
            match store.resampling_quality() {
                Err(Error::InvalidSetting { key, .. }) => Some(key),
                Err(error) => panic!("{error}"),
                Ok(_) => None,
            }
        };
        assert_eq!(key("resampling = \"best\"\n"), Some("resampling"));
        assert_eq!(
            key("resampling = \"custom\"\nresampling_fft_size = 300\n"),
            Some("resampling_fft_size")
        );
        assert_eq!(
            key("resampling = \"custom\"\nresampling_window = \"box\"\n"),
            Some("resampling_window")
        );
        assert_eq!(
            key("resampling = \"high\"\nresampling_fft_size = 300\n"),
            None
        );
    }

    #[test]
    fn saving_one_output_key_leaves_the_other_alone() {
        let store = temporary_store("output-one-key");
        store.save_output_device(Some("a:b")).unwrap();
        store.save_sample_rate(SampleRate::Fixed(44_100)).unwrap();

        store.save_sample_rate(SampleRate::MatchSong).unwrap();
        assert_eq!(read(&store, &[]).output_device.as_deref(), Some("a:b"));

        store.save_sample_rate(SampleRate::Fixed(96_000)).unwrap();
        store.save_output_device(None).unwrap();
        assert_eq!(store.sample_rate().unwrap(), SampleRate::Fixed(96_000));
    }

    #[test]
    fn saving_an_output_key_leaves_a_key_it_cannot_read_alone() {
        let store = temporary_store("output-unreadable");
        std::fs::create_dir_all(store.dir()).unwrap();
        std::fs::write(
            store.config_path(),
            "volume = \"loud\"\noutput_device = \"a:b\"\n",
        )
        .unwrap();
        store.save_sample_rate(SampleRate::Fixed(96_000)).unwrap();
        let text = std::fs::read_to_string(store.config_path()).unwrap();
        assert!(text.contains("volume = \"loud\""), "{text}");
        assert!(text.contains("output_device = \"a:b\""), "{text}");
        assert!(text.contains("sample_rate = 96000"), "{text}");
    }

    #[test]
    fn environment_overrides_the_saved_output() {
        let store = temporary_store("output-environment");
        store.save_output_device(Some("a:b")).unwrap();
        store.save_sample_rate(SampleRate::Fixed(44_100)).unwrap();
        let settings = read(
            &store,
            &[
                ("HIPHILE_OUTPUT_DEVICE", "c:d"),
                ("HIPHILE_SAMPLE_RATE", "48000"),
            ],
        );
        assert_eq!(settings.output_device.as_deref(), Some("c:d"));
        assert_eq!(
            store.parse_sample_rate(&settings).unwrap(),
            SampleRate::Fixed(48_000)
        );
        let settings = read(&store, &[("HIPHILE_SAMPLE_RATE", "x4")]);
        assert_eq!(
            store.parse_sample_rate(&settings).unwrap(),
            SampleRate::SongMultiple(4)
        );
    }

    #[test]
    fn song_cache_capacity_is_read_in_gib() {
        let store = temporary_store("song-cache");
        let capacity = |env: &[(&str, &str)]| {
            store
                .cache_bytes("song_cache_gib", read(&store, env).song_cache_gib)
                .map_err(|error| error.to_string())
        };
        assert_eq!(capacity(&[]), Ok(None));
        std::fs::create_dir_all(store.dir()).unwrap();
        std::fs::write(store.config_path(), "song_cache_gib = 2\n").unwrap();
        assert_eq!(capacity(&[]), Ok(Some(2 << 30)));
        assert_eq!(
            capacity(&[("HIPHILE_SONG_CACHE_GIB", "0.5")]),
            Ok(Some(512 << 20))
        );
        assert_eq!(
            capacity(&[("HIPHILE_SONG_CACHE_GIB", "0")]),
            Ok(Some(u64::MAX))
        );
        let negative = capacity(&[("HIPHILE_SONG_CACHE_GIB", "-1")]).unwrap_err();
        assert!(negative.contains("HIPHILE_SONG_CACHE_GIB"), "{negative}");
    }

    #[test]
    fn song_cache_gib_round_trips_and_clears() {
        let store = temporary_store("song-cache-save");
        store.save_volume(0.5).unwrap();

        store.save_song_cache_gib(Some(0.5)).unwrap();
        assert_eq!(read(&store, &[]).song_cache_gib, Some(0.5));
        assert_eq!(
            store
                .cache_bytes("song_cache_gib", read(&store, &[]).song_cache_gib)
                .unwrap(),
            Some(512 << 20)
        );

        store.save_song_cache_gib(Some(0.0)).unwrap();
        assert_eq!(read(&store, &[]).song_cache_gib, Some(0.0));

        store.save_song_cache_gib(None).unwrap();
        let settings = read(&store, &[]);
        assert_eq!(settings.song_cache_gib, None);
        assert_eq!(settings.volume, Some(0.5));
    }

    #[test]
    fn cover_cache_gib_is_kept_apart_from_the_song_cache() {
        let store = temporary_store("cover-cache-save");
        store.save_song_cache_gib(Some(2.0)).unwrap();

        store.save_cover_cache_gib(Some(0.5)).unwrap();
        assert_eq!(store.cover_cache_gib().unwrap(), Some(0.5));
        assert_eq!(store.cover_cache_capacity().unwrap(), Some(512 << 20));
        assert_eq!(store.song_cache_gib().unwrap(), Some(2.0));

        store.save_cover_cache_gib(None).unwrap();
        assert_eq!(store.cover_cache_capacity().unwrap(), None);
        assert_eq!(store.song_cache_gib().unwrap(), Some(2.0));
    }

    #[test]
    fn a_negative_cover_cache_limit_is_an_error_naming_it() {
        let store = temporary_store("cover-cache-negative");
        std::fs::create_dir_all(store.config_path().parent().unwrap()).unwrap();
        std::fs::write(store.config_path(), "cover_cache_gib = -1\n").unwrap();
        let error = store.cover_cache_capacity().unwrap_err().to_string();
        assert!(error.contains("cover_cache_gib"), "{error}");
    }

    #[test]
    fn appearance_round_trips_and_clears() {
        let store = temporary_store("appearance");
        store.save_volume(0.5).unwrap();
        let appearance = |env: &[(&str, &str)]| {
            store
                .parse_appearance(read(&store, env).appearance.as_deref())
                .unwrap()
        };
        assert_eq!(appearance(&[]), None);

        store.save_appearance(Some(Appearance::Dark)).unwrap();
        assert_eq!(appearance(&[]), Some(Appearance::Dark));
        store.save_appearance(Some(Appearance::Light)).unwrap();
        assert_eq!(appearance(&[]), Some(Appearance::Light));

        store.save_appearance(None).unwrap();
        assert_eq!(appearance(&[]), None);
        assert_eq!(read(&store, &[]).volume, Some(0.5));
    }

    #[test]
    fn environment_overrides_the_saved_appearance() {
        let store = temporary_store("appearance-environment");
        store.save_appearance(Some(Appearance::Light)).unwrap();
        let appearance = |value| {
            store
                .parse_appearance(
                    read(&store, &[("HIPHILE_APPEARANCE", value)])
                        .appearance
                        .as_deref(),
                )
                .unwrap()
        };
        assert_eq!(appearance("dark"), Some(Appearance::Dark));
        assert_eq!(appearance("system"), None);
    }

    #[test]
    fn an_unknown_appearance_is_an_error_that_leaves_other_settings_readable() {
        let store = temporary_store("appearance-unknown");
        std::fs::create_dir_all(store.dir()).unwrap();
        std::fs::write(store.config_path(), "appearance = \"blue\"\nvolume = 0.5\n").unwrap();
        let settings = read(&store, &[]);
        assert_eq!(settings.volume, Some(0.5));
        assert!(matches!(
            store.parse_appearance(settings.appearance.as_deref()),
            Err(Error::InvalidSetting {
                key: "appearance",
                ..
            })
        ));
    }

    #[test]
    fn romaji_search_round_trips_and_is_on_unless_told_otherwise() {
        let store = temporary_store("romaji-search");
        store.save_volume(0.5).unwrap();
        let is_on = |env: &[(&str, &str)]| read(&store, env).romaji_search.unwrap_or(true);
        assert!(is_on(&[]));

        store.save_romaji_search(false).unwrap();
        assert!(!is_on(&[]));
        assert!(is_on(&[("HIPHILE_ROMAJI_SEARCH", "true")]));
    }

    #[test]
    fn discord_presence_round_trips_and_is_on_unless_told_otherwise() {
        let store = temporary_store("discord-presence");
        store.save_volume(0.5).unwrap();
        let is_on = |env: &[(&str, &str)]| read(&store, env).discord_presence.unwrap_or(true);
        assert!(is_on(&[]));

        store.save_discord_presence(false).unwrap();
        assert!(!is_on(&[]));
        assert!(is_on(&[("HIPHILE_DISCORD_PRESENCE", "true")]));
    }

    #[test]
    fn auto_update_round_trips_and_is_on_unless_told_otherwise() {
        let store = temporary_store("auto-update");
        store.save_volume(0.5).unwrap();
        let is_on = |env: &[(&str, &str)]| read(&store, env).auto_update.unwrap_or(true);
        assert!(is_on(&[]));

        store.save_auto_update(false).unwrap();
        assert!(!is_on(&[]));
        assert!(is_on(&[("HIPHILE_AUTO_UPDATE", "true")]));
    }

    #[test]
    fn lyrics_highlight_round_trips_and_is_by_word_unless_told_otherwise() {
        let store = temporary_store("lyrics-highlight");
        store.save_volume(0.5).unwrap();
        let highlight = |env: &[(&str, &str)]| {
            store
                .parse_lyrics_highlight(read(&store, env).lyrics_highlight.as_deref())
                .unwrap()
        };
        assert_eq!(highlight(&[]), LyricsHighlight::ByWord);

        store
            .save_lyrics_highlight(LyricsHighlight::ByLine)
            .unwrap();
        assert_eq!(highlight(&[]), LyricsHighlight::ByLine);
        assert_eq!(
            highlight(&[("HIPHILE_LYRICS_HIGHLIGHT", "word")]),
            LyricsHighlight::ByWord
        );

        store
            .save_lyrics_highlight(LyricsHighlight::ByWord)
            .unwrap();
        assert_eq!(highlight(&[]), LyricsHighlight::ByWord);
        assert_eq!(read(&store, &[]).volume, Some(0.5));
    }

    #[test]
    fn an_unknown_lyrics_highlight_is_an_error() {
        let store = temporary_store("lyrics-highlight-unknown");
        std::fs::create_dir_all(store.dir()).unwrap();
        std::fs::write(store.config_path(), "lyrics_highlight = \"note\"\n").unwrap();
        let settings = read(&store, &[]);
        assert!(matches!(
            store.parse_lyrics_highlight(settings.lyrics_highlight.as_deref()),
            Err(Error::InvalidSetting {
                key: "lyrics_highlight",
                ..
            })
        ));
    }

    #[test]
    fn a_password_in_the_file_is_only_for_the_files_profile() {
        let store = temporary_store("password");
        let alice = Profile::new("https://music.example.com", "alice");
        assert_eq!(store.password(&alice).unwrap(), None);

        std::fs::create_dir_all(store.dir()).unwrap();
        std::fs::write(
            store.config_path(),
            "server = \"https://music.example.com\"\nusername = \"alice\"\npassword = \"sesame\"\n",
        )
        .unwrap();
        assert_eq!(store.password(&alice).unwrap().as_deref(), Some("sesame"));
        for other in [
            Profile::new("https://music.example.com", "bob"),
            Profile::new("https://other.example.com", "alice"),
        ] {
            assert_eq!(store.password(&other).unwrap(), None);
        }
    }

    #[test]
    fn each_profile_saved_is_kept_once_in_the_order_first_saved() {
        let store = temporary_store("profiles");
        let alice = Profile::new("https://music.example.com", "alice");
        let bob = Profile::new("https://other.example.com", "bob");
        assert_eq!(store.profiles().unwrap(), []);

        store.save_profile(&alice).unwrap();
        store.save_profile(&bob).unwrap();
        store.save_profile(&alice).unwrap();

        assert_eq!(store.profiles().unwrap(), [alice.clone(), bob]);
        assert_eq!(load(&store, &[]), Some(alice));
    }

    #[test]
    fn a_forgotten_profile_is_no_longer_among_them_but_stays_the_last() {
        let store = temporary_store("profiles-forgotten");
        let alice = Profile::new("https://music.example.com", "alice");
        let bob = Profile::new("https://music.example.com", "bob");
        store.save_profile(&bob).unwrap();
        store.save_profile(&alice).unwrap();

        store.forget_profile(&alice).unwrap();
        assert_eq!(store.profiles().unwrap(), std::slice::from_ref(&bob));
        assert_eq!(load(&store, &[]), Some(alice));

        store.forget_profile(&bob).unwrap();
        assert_eq!(store.profiles().unwrap(), []);
        let text = std::fs::read_to_string(store.config_path()).unwrap();
        assert!(!text.contains("profiles"), "{text}");
    }

    #[test]
    fn a_key_saved_after_the_profiles_is_read_as_a_setting() {
        let store = temporary_store("profiles-then-volume");
        let alice = Profile::new("https://music.example.com", "alice");
        store.save_profile(&alice).unwrap();

        store.save_volume(0.5).unwrap();

        assert_eq!(read(&store, &[]).volume, Some(0.5));
        assert_eq!(store.profiles().unwrap(), [alice]);
    }

    #[test]
    fn profiles_written_inline_are_read_and_kept_when_another_is_saved() {
        let store = temporary_store("profiles-inline");
        std::fs::create_dir_all(store.dir()).unwrap();
        std::fs::write(
            store.config_path(),
            "profiles = [{ server = \"https://music.example.com\", username = \"alice\" }]\n",
        )
        .unwrap();
        let alice = Profile::new("https://music.example.com", "alice");
        assert_eq!(store.profiles().unwrap(), std::slice::from_ref(&alice));

        let bob = Profile::new("https://other.example.com", "bob");
        store.save_profile(&bob).unwrap();

        assert_eq!(store.profiles().unwrap(), [alice, bob]);
    }

    #[test]
    fn profiles_written_inline_stay_so_with_their_comments() {
        let store = temporary_store("profiles-inline-comments");
        std::fs::create_dir_all(store.dir()).unwrap();
        let written = "profiles = [\n  # Home\n  { server = \"https://music.example.com\", username = \"alice\" }, # Me\n]\n";
        std::fs::write(store.config_path(), written).unwrap();
        let alice = Profile::new("https://music.example.com", "alice");
        let text = || std::fs::read_to_string(store.config_path()).unwrap();

        store.save_profile(&alice).unwrap();
        assert!(text().contains(written), "{}", text());

        let bob = Profile::new("https://other.example.com", "bob");
        store.save_profile(&bob).unwrap();
        assert!(
            text().contains(
                "  # Home\n  { server = \"https://music.example.com\", username = \"alice\" }, # Me\n  { server = \"https://other.example.com\", username = \"bob\" },\n]"
            ),
            "{}",
            text()
        );
        assert!(!text().contains("[[profiles"), "{}", text());

        store.forget_profile(&alice).unwrap();
        assert_eq!(store.profiles().unwrap(), [bob]);
        assert!(text().starts_with("profiles = ["), "{}", text());
    }

    #[test]
    fn a_profile_written_wrong_is_left_out_and_the_rest_still_read() {
        let store = temporary_store("profiles-incomplete");
        std::fs::create_dir_all(store.dir()).unwrap();
        std::fs::write(
            store.config_path(),
            "volume = 0.5\n\n[[profiles]]\nserver = \"https://music.example.com\"\n\n[[profiles]]\nserver = \"https://other.example.com\"\nusername = \"bob\"\n",
        )
        .unwrap();
        let bob = Profile::new("https://other.example.com", "bob");

        assert_eq!(store.profiles().unwrap(), std::slice::from_ref(&bob));
        assert_eq!(store.volume().unwrap(), Some(0.5));

        let alice = Profile::new("https://music.example.com", "alice");
        store.save_profile(&alice).unwrap();
        store.forget_profile(&bob).unwrap();
        assert_eq!(store.profiles().unwrap(), [alice]);
        let text = std::fs::read_to_string(store.config_path()).unwrap();
        assert_eq!(text.matches("[[profiles]]").count(), 2, "{text}");
    }

    #[test]
    fn profiles_written_as_something_else_are_none_and_left_as_they_are() {
        let store = temporary_store("profiles-table");
        std::fs::create_dir_all(store.dir()).unwrap();
        let written = "[profiles]\nserver = \"https://music.example.com\"\nusername = \"alice\"\n";
        std::fs::write(store.config_path(), written).unwrap();

        assert_eq!(store.profiles().unwrap(), []);
        let bob = Profile::new("https://other.example.com", "bob");
        store.save_profile(&bob).unwrap();
        store.forget_profile(&bob).unwrap();

        assert_eq!(load(&store, &[]), Some(bob));
        let text = std::fs::read_to_string(store.config_path()).unwrap();
        assert!(text.ends_with(written), "{text}");
    }

    #[test]
    fn a_server_is_the_same_whatever_the_case_of_its_host_or_the_slashes_at_its_end() {
        assert_eq!(
            Profile::new("HTTPS://Music.Example.com/", "alice"),
            Profile::new("https://music.example.com", "alice"),
        );
        assert_eq!(
            Profile::new("https://music.example.com/Navidrome/", "alice").server(),
            "https://music.example.com/Navidrome",
        );
        assert_ne!(
            Profile::new("https://music.example.com", "Alice"),
            Profile::new("https://music.example.com", "alice"),
        );
    }

    #[test]
    fn a_password_written_for_the_server_with_a_slash_at_its_end_is_still_its() {
        let store = temporary_store("password-slash");
        std::fs::create_dir_all(store.dir()).unwrap();
        std::fs::write(
            store.config_path(),
            "server = \"https://music.example.com/\"\nusername = \"alice\"\npassword = \"sesame\"\n",
        )
        .unwrap();
        let alice = Profile::new("https://music.example.com", "alice");
        assert_eq!(load(&store, &[]), Some(alice.clone()));
        assert_eq!(store.password(&alice).unwrap().as_deref(), Some("sesame"));

        store.save_profile(&alice).unwrap();
        assert_eq!(store.password(&alice).unwrap().as_deref(), Some("sesame"));
    }

    #[test]
    fn saving_the_same_profile_keeps_the_password_and_another_removes_it() {
        let store = temporary_store("password-saved");
        let alice = Profile::new("https://music.example.com", "alice");
        std::fs::create_dir_all(store.dir()).unwrap();
        std::fs::write(
            store.config_path(),
            "server = \"https://music.example.com\"\nusername = \"alice\"\npassword = \"sesame\"\n",
        )
        .unwrap();

        assert_eq!(store.save_profile(&alice).unwrap(), None);
        assert_eq!(store.password(&alice).unwrap().as_deref(), Some("sesame"));

        let bob = Profile::new("https://music.example.com", "bob");
        assert_eq!(
            store.save_profile(&bob).unwrap(),
            Some((alice.clone(), "sesame".to_owned()))
        );
        assert_eq!(store.password(&alice).unwrap(), None);
        assert_eq!(store.password(&bob).unwrap(), None);
        assert_eq!(store.save_profile(&alice).unwrap(), None);
        assert_eq!(read(&store, &[]).password, None);
    }

    #[test]
    fn concurrent_saves_keep_every_change() {
        let store = temporary_store("concurrent");
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let store = store.clone();
                std::thread::spawn(move || {
                    for _ in 0..20 {
                        if i == 0 {
                            store
                                .save_profile(&Profile::new("https://music.example.com", "alice"))
                                .unwrap();
                        } else {
                            store.save_volume(0.5).unwrap();
                        }
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        let settings = read(&store, &[]);
        assert_eq!(settings.volume, Some(0.5));
        assert_eq!(
            settings.server.as_deref(),
            Some("https://music.example.com")
        );
        let leftovers = std::fs::read_dir(store.dir())
            .unwrap()
            .filter(|entry| entry.as_ref().unwrap().path() != store.config_path())
            .count();
        assert_eq!(leftovers, 0, "temporary files were left behind");
    }

    #[test]
    fn invalid_toml_is_reported_and_left_alone() {
        let store = temporary_store("invalid");
        std::fs::create_dir_all(store.dir()).unwrap();
        std::fs::write(store.config_path(), "server = ").unwrap();
        assert!(matches!(
            store.read(
                config_rs::Environment::with_prefix(ENV_PREFIX).source(Some(Default::default()))
            ),
            Err(Error::Read { .. })
        ));
        assert!(matches!(
            store.save_profile(&Profile::new("https://music.example.com", "alice")),
            Err(Error::Unparsable { .. })
        ));
        assert_eq!(
            std::fs::read_to_string(store.config_path()).unwrap(),
            "server = "
        );
    }

    #[test]
    fn the_imgbb_key_is_saved_read_and_removed() {
        let store = temporary_store("imgbb-key");
        assert_eq!(read(&store, &[]).imgbb_api_key, None);

        store.save_imgbb_api_key(Some("abc123")).unwrap();
        assert_eq!(read(&store, &[]).imgbb_api_key.as_deref(), Some("abc123"));

        store.save_imgbb_api_key(None).unwrap();
        assert_eq!(read(&store, &[]).imgbb_api_key, None);
    }

    #[test]
    fn the_environment_overrides_the_imgbb_key() {
        let store = temporary_store("imgbb-key-env");
        store.save_imgbb_api_key(Some("file")).unwrap();

        let settings = read(&store, &[("HIPHILE_IMGBB_API_KEY", "env")]);

        assert_eq!(settings.imgbb_api_key.as_deref(), Some("env"));
    }

    #[test]
    fn a_blank_imgbb_key_is_no_key() {
        assert_eq!(usable_key(None), None);
        assert_eq!(usable_key(Some("  ".to_owned())), None);
        assert_eq!(
            usable_key(Some(" abc\n".to_owned())).as_deref(),
            Some("abc")
        );
    }
}
