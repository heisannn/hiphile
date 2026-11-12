//! Server profiles and passwords shared by the GUI and the CLI.
//!
//! Settings live in `~/.config/hiphile/config.toml` on every platform.
//! Passwords live in the system keychain, unless the user wrote one in the
//! file by hand.

mod keychain;
mod profile;

pub use keychain::Keychain;
pub use profile::{
    Appearance, BALANCE_DBS, CROSSFEED_CUTOFFS_HZ, CROSSFEED_SEPARATIONS_DB, Crossfeed,
    CrossfeedFilter, CrossfeedPreset, DSD_ORDERS, DSD_SINC_LENGTHS, DsdFilter, DsdOutput,
    DsdParameters, DsdQuality, DsdRate, LONGEST_CROSSFADE_SECONDS, LyricsHighlight, Pane, Profile,
    R8BRAIN_ATTENUATIONS, R8BRAIN_CACHE_SIZES, R8BRAIN_FILTER_FRACTIONS, R8BRAIN_MAX_INPUT_FRAMES,
    R8BRAIN_TRANSITION_BANDS, R8brainFilter, R8brainParameters, R8brainPerformance, R8brainPhase,
    R8brainQuality, RESAMPLING_FFT_SIZES, ResamplingQuality, ResamplingWindow,
    SAMPLE_RATE_MULTIPLES, SOXR_COEF_TABLE_KBS, SOXR_LARGE_DFT_SIZES, SOXR_MIN_DFT_SIZES,
    SOXR_PASSBAND_ENDS, SOXR_PRECISIONS, SampleRate, SoxrFilter, SoxrInterpolation, SoxrParameters,
    SoxrPerformance, SoxrPhase, SoxrQuality, Store, ThemeColor, cache_capacity_of,
};

use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the home directory is unknown, so there is nowhere to keep settings")]
    NoHomeDir,
    #[error("could not read the settings in {}: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: config_rs::ConfigError,
    },
    #[error("could not write {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{} is not valid TOML, so it was left unchanged: {source}", path.display())]
    Unparsable {
        path: PathBuf,
        #[source]
        source: toml_edit::TomlError,
    },
    #[error("`{key}` must be {expected}; check {} and HIPHILE_{}", path.display(), key.to_ascii_uppercase())]
    InvalidSetting {
        path: PathBuf,
        key: &'static str,
        expected: &'static str,
    },
    #[error("the system keychain failed: {0}")]
    Keychain(#[source] keyring_core::Error),
    /// One failure handed to several owners, as the others cannot be cloned.
    #[error(transparent)]
    Shared(Arc<Error>),
}
