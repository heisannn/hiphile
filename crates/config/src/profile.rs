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
    pub fn new(server: impl Into<String>, username: impl Into<String>) -> Self {
        Self {
            server: server.into(),
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

pub const RESAMPLING_FFT_SIZES: [u32; 7] = [256, 512, 1024, 2048, 4096, 8192, 16384];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResamplingQuality {
    #[default]
    Medium,
    High,
    Max,
    Custom {
        fft_size: u32,
        window: ResamplingWindow,
    },
}

impl ResamplingQuality {
    pub fn fft_size(self) -> u32 {
        match self {
            Self::Medium => 256,
            Self::High => 1024,
            Self::Max => 4096,
            Self::Custom { fft_size, .. } => fft_size,
        }
    }

    pub fn window(self) -> ResamplingWindow {
        match self {
            Self::Medium | Self::High | Self::Max => ResamplingWindow::BlackmanHarris2,
            Self::Custom { window, .. } => window,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Medium => "medium",
            Self::High => "high",
            Self::Max => "max",
            Self::Custom { .. } => "custom",
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
    sample_rate: Option<u32>,
    resampling: Option<String>,
    resampling_fft_size: Option<u32>,
    resampling_window: Option<String>,
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

/// The settings directory and its `config.toml`:
///
/// ```toml
/// server = "https://music.example.com"
/// username = "alice"
/// volume = 0.8
/// song_cache_gib = 8
/// cover_cache_gib = 8
/// output_device = "coreaudio:BuiltInSpeakerDevice"
/// sample_rate = 48000
/// resampling = "custom"
/// resampling_fft_size = 2048
/// resampling_window = "hann-2"
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

    /// The password written by hand in the file, if it belongs to `profile`.
    /// It is only for the file's own `server` and `username`, so it is never
    /// sent to a server named by an option or the environment. The app never
    /// writes this key.
    pub fn password(&self, profile: &Profile) -> Result<Option<String>, Error> {
        let settings = self.read(
            config_rs::Environment::with_prefix(ENV_PREFIX).source(Some(Default::default())),
        )?;
        let same_profile = settings.server.as_deref() == Some(profile.server())
            && settings.username.as_deref() == Some(profile.username());
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

    /// The rate in Hz to run the output device at, if one was chosen rather than each song's
    /// own.
    pub fn sample_rate(&self) -> Result<Option<u32>, Error> {
        let settings = self.read(config_rs::Environment::with_prefix(ENV_PREFIX))?;
        Ok(settings.sample_rate.filter(|rate| *rate > 0))
    }

    /// Medium unless the file says otherwise. The FFT size and window are only read for
    /// `custom`, and default to those of medium.
    pub fn resampling_quality(&self) -> Result<ResamplingQuality, Error> {
        let settings = self.read(config_rs::Environment::with_prefix(ENV_PREFIX))?;
        self.parse_resampling_quality(&settings)
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
                "\"medium\", \"high\", \"max\" or \"custom\"",
            )),
        }
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

    /// Records `profile` in `config.toml`, keeping the file's other keys and
    /// comments. A different profile removes `password`, which belonged to the
    /// one it replaces.
    pub fn save_profile(&self, profile: &Profile) -> Result<(), Error> {
        self.edit(|document| {
            let saved = |key| document.get(key).and_then(toml_edit::Item::as_str);
            if saved("server") != Some(profile.server())
                || saved("username") != Some(profile.username())
            {
                document.remove("password");
            }
            document["server"] = toml_edit::value(profile.server());
            document["username"] = toml_edit::value(profile.username());
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

    /// `None` removes the key, which means each song's own rate.
    pub fn save_sample_rate(&self, sample_rate: Option<u32>) -> Result<(), Error> {
        self.edit(|document| match sample_rate {
            Some(rate) => document["sample_rate"] = toml_edit::value(i64::from(rate)),
            None => {
                document.remove("sample_rate");
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
        store.save_sample_rate(Some(96_000)).unwrap();
        let settings = read(&store, &[]);
        assert_eq!(settings.output_device.as_deref(), Some("coreaudio:device"));
        assert_eq!(settings.sample_rate, Some(96_000));

        store.save_output_device(None).unwrap();
        store.save_sample_rate(None).unwrap();
        let settings = read(&store, &[]);
        assert_eq!(settings.output_device, None);
        assert_eq!(settings.sample_rate, None);
        assert_eq!(load(&store, &[]), Some(profile));
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
        store.save_sample_rate(Some(44_100)).unwrap();

        store.save_sample_rate(None).unwrap();
        assert_eq!(read(&store, &[]).output_device.as_deref(), Some("a:b"));

        store.save_sample_rate(Some(96_000)).unwrap();
        store.save_output_device(None).unwrap();
        assert_eq!(read(&store, &[]).sample_rate, Some(96_000));
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
        store.save_sample_rate(Some(96_000)).unwrap();
        let text = std::fs::read_to_string(store.config_path()).unwrap();
        assert!(text.contains("volume = \"loud\""), "{text}");
        assert!(text.contains("output_device = \"a:b\""), "{text}");
        assert!(text.contains("sample_rate = 96000"), "{text}");
    }

    #[test]
    fn environment_overrides_the_saved_output() {
        let store = temporary_store("output-environment");
        store.save_output_device(Some("a:b")).unwrap();
        store.save_sample_rate(Some(44_100)).unwrap();
        let settings = read(
            &store,
            &[
                ("HIPHILE_OUTPUT_DEVICE", "c:d"),
                ("HIPHILE_SAMPLE_RATE", "48000"),
            ],
        );
        assert_eq!(settings.output_device.as_deref(), Some("c:d"));
        assert_eq!(settings.sample_rate, Some(48_000));
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
    fn saving_the_same_profile_keeps_the_password_and_another_removes_it() {
        let store = temporary_store("password-saved");
        let alice = Profile::new("https://music.example.com", "alice");
        std::fs::create_dir_all(store.dir()).unwrap();
        std::fs::write(
            store.config_path(),
            "server = \"https://music.example.com\"\nusername = \"alice\"\npassword = \"sesame\"\n",
        )
        .unwrap();

        store.save_profile(&alice).unwrap();
        assert_eq!(store.password(&alice).unwrap().as_deref(), Some("sesame"));

        let bob = Profile::new("https://music.example.com", "bob");
        store.save_profile(&bob).unwrap();
        assert_eq!(store.password(&bob).unwrap(), None);
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
