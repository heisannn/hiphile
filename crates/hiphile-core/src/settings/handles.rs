use std::sync::Arc;

use executor::Executor;

use crate::hiphile::share;

use super::{
    AppearanceSettings, AudioSettings, CacheKind, CacheLimit, DiscordSettings, DspSettings,
    LyricsSettings, QueueSettings,
};

/// Every setting the app keeps, each a handle of its own that reads its part of one store.
#[derive(Clone)]
pub struct Settings {
    appearance: AppearanceSettings,
    audio: AudioSettings,
    song_cache: CacheLimit,
    cover_cache: CacheLimit,
    discord: DiscordSettings,
    dsp: DspSettings,
    lyrics: LyricsSettings,
    queue: QueueSettings,
}

impl Settings {
    pub fn new(executor: Executor, store: Result<config::Store, config::Error>) -> Self {
        let store = store.map_err(Arc::new);
        Self {
            appearance: AppearanceSettings::new(executor.clone(), share(&store)),
            audio: AudioSettings::new(executor.clone(), share(&store)),
            song_cache: CacheLimit::new(CacheKind::Songs, executor.clone(), share(&store)),
            cover_cache: CacheLimit::new(CacheKind::Covers, executor.clone(), share(&store)),
            discord: DiscordSettings::new(executor.clone(), share(&store)),
            dsp: DspSettings::new(executor.clone(), share(&store)),
            lyrics: LyricsSettings::new(executor.clone(), share(&store)),
            queue: QueueSettings::new(executor, share(&store)),
        }
    }

    pub fn appearance(&self) -> &AppearanceSettings {
        &self.appearance
    }

    pub fn audio(&self) -> &AudioSettings {
        &self.audio
    }

    pub fn song_cache(&self) -> &CacheLimit {
        &self.song_cache
    }

    pub fn cover_cache(&self) -> &CacheLimit {
        &self.cover_cache
    }

    pub fn discord(&self) -> &DiscordSettings {
        &self.discord
    }

    pub fn dsp(&self) -> &DspSettings {
        &self.dsp
    }

    pub fn lyrics(&self) -> &LyricsSettings {
        &self.lyrics
    }

    pub fn queue(&self) -> &QueueSettings {
        &self.queue
    }

    /// Writes what is still settling or waiting, which nothing would write once the app has
    /// quit.
    pub fn save_before_quitting(&self) {
        self.audio.save_before_quitting();
        self.song_cache.save_before_quitting();
        self.cover_cache.save_before_quitting();
        self.discord.save_before_quitting();
        self.dsp.save_before_quitting();
    }
}
