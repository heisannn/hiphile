//! The app's settings. Each saves itself and publishes a snapshot of what it holds, which the
//! host passes on to those who use it.

mod appearance;
mod audio;
mod cache_limit;
mod discord;
mod freshness;
mod handles;
mod lyrics;
mod number_range;
mod queue;
mod song_cache_usage;

pub use appearance::{APPEARANCES, AppearanceSettings, AppearanceSnapshot};
pub use audio::{
    AudioSettings, AudioSnapshot, Choice, DsdWarning, MATCH_SONG, PcmSetting, R8BRAIN_PHASES,
    R8BRAIN_QUALITIES, R8brainNumber, R8brainSetting, RESAMPLERS, RUBATO_QUALITIES,
    SOXR_INTERPOLATIONS, SOXR_PHASES, SOXR_QUALITIES, SYSTEM_DEFAULT, SoxrSetting,
};
pub use cache_limit::{CacheKind, CacheLimit, CacheLimitSnapshot};
pub use discord::{DiscordSettings, DiscordSnapshot};
pub use freshness::Freshness;
pub use handles::Settings;
pub use lyrics::{LYRICS_HIGHLIGHTS, LyricsSettings, LyricsSnapshot};
pub use number_range::NumberRange;
pub use queue::{QueueSettings, QueueSnapshot};
pub use song_cache_usage::{SongCacheSnapshot, SongCacheUsage, format_size};
