//! The app's settings. Each saves itself and publishes a snapshot of what it holds, which the
//! host passes on to those who use it.

mod appearance;
mod audio;
mod cache_limit;
mod discord;
mod lyrics;
mod queue;
mod song_cache_usage;

pub use appearance::{APPEARANCES, AppearanceSettings, AppearanceSnapshot};
pub use audio::{
    AudioSettings, AudioSnapshot, Choice, MATCH_SONG, R8BRAIN_PHASES, R8BRAIN_QUALITIES,
    RESAMPLERS, RUBATO_QUALITIES, SOXR_INTERPOLATIONS, SOXR_PHASES, SOXR_QUALITIES, SYSTEM_DEFAULT,
};
pub use cache_limit::{CacheKind, CacheLimit, CacheLimitSnapshot};
pub use discord::{DiscordSettings, DiscordSnapshot};
pub use lyrics::{LYRICS_HIGHLIGHTS, LyricsSettings, LyricsSnapshot};
pub use queue::{QueueSettings, QueueSnapshot};
pub use song_cache_usage::{SongCacheSnapshot, SongCacheUsage, format_size};
