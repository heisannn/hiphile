use std::time::Duration;

use gpui_kit::{AppContext as _, Context, EventEmitter, SharedString, Subscription, Task};

/// What the settings keep when they leave the limit unset.
pub(crate) const DEFAULT_GIB: f64 = (catalog::SONG_CAPACITY >> 30) as f64;
/// The most the field accepts, far beyond any disk.
pub(crate) const MAX_GIB: f64 = (1u32 << 20) as f64;
/// How long the limit must stay unchanged before it applies. The field reports
/// every keystroke, and on the way to `16` a limit of `1` could remove songs.
const SETTLE: Duration = Duration::from_millis(500);

/// The limit changed; read it from [`SongCacheSettings::capacity`].
pub struct SongCacheChanged;

/// How many GiB of songs may be kept on this computer, where `0` means no
/// limit. Saving happens off the main thread.
pub struct SongCacheSettings {
    /// `None` without a home directory: the limit then lasts until quitting.
    store: Option<config::Store>,
    /// What the field last held as a number. `None` means the default limit.
    gib: Option<f64>,
    /// The limit in effect, which `gib` becomes once it settles.
    applied: Option<f64>,
    /// Whether no songs are being kept, because the limit could not be read.
    /// No limit is in effect then, so `applied` means nothing.
    keeps_none: bool,
    error: Option<SharedString>,
    _settle: Task<()>,
    unsaved: bool,
    saving: bool,
    _save: Task<()>,
    _quit: Subscription,
}

impl EventEmitter<SongCacheChanged> for SongCacheSettings {}

impl SongCacheSettings {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let saved = read_saved_limit();
        Self {
            store: saved.store,
            gib: saved.gib,
            applied: saved.gib,
            keeps_none: saved.keeps_none,
            error: saved.error,
            _settle: Task::ready(()),
            unsaved: false,
            saving: false,
            _save: Task::ready(()),
            _quit: cx.on_app_quit(|this, _| {
                this.save_before_quitting();
                async {}
            }),
        }
    }

    /// The bytes to keep, or `None` for the default limit.
    pub fn capacity(&self) -> Option<u64> {
        self.applied.and_then(config::song_cache_capacity_of)
    }

    pub(crate) fn gib(&self) -> f64 {
        self.gib.unwrap_or(DEFAULT_GIB)
    }

    pub(crate) fn error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }

    pub(crate) fn set_gib(&mut self, gib: f64, cx: &mut Context<Self>) {
        self.gib = chosen_limit(gib);
        cx.notify();
        self._settle = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SETTLE).await;
            this.update(cx, |this, cx| this.apply(cx)).ok();
        });
    }

    /// Goes back to the limit in effect, dropping a change that has not
    /// settled, as when the field is emptied on the way to another number.
    pub(crate) fn revert(&mut self, cx: &mut Context<Self>) {
        self._settle = Task::ready(());
        if self.gib != self.applied {
            self.gib = self.applied;
            cx.notify();
        }
    }

    fn apply(&mut self, cx: &mut Context<Self>) {
        if self.gib == self.applied && !self.keeps_none {
            return;
        }
        self.applied = self.gib;
        self.keeps_none = false;
        if self.store.is_some() {
            self.error = None;
        }
        cx.emit(SongCacheChanged);
        cx.notify();
        self.unsaved = true;
        self.save(cx);
    }

    /// One write at a time, so a slow one cannot leave an older limit on
    /// disk; the latest is written when the current write finishes.
    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        if self.saving {
            return;
        }
        self.saving = true;
        self._save = cx.spawn(async move |this, cx| {
            while let Ok(Some(gib)) = this.update(cx, |this, _| {
                this.saving = std::mem::take(&mut this.unsaved);
                this.saving.then_some(this.applied)
            }) {
                let store = store.clone();
                let saved = cx
                    .background_spawn(async move { store.save_song_cache_gib(gib) })
                    .await;
                this.update(cx, |this, cx| {
                    this.error = saved.err().map(|error| {
                        format!("Couldn't save the song cache limit: {error}.").into()
                    });
                    cx.notify();
                })
                .ok();
            }
        });
    }

    /// Writes a limit still settling or waiting for its turn, which no task
    /// would write once the app has quit.
    fn save_before_quitting(&self) {
        if (self.gib != self.applied || self.unsaved)
            && let Some(store) = &self.store
        {
            store.save_song_cache_gib(self.gib).ok();
        }
    }
}

/// The limit for `gib` typed into the field. The default leaves the setting
/// unset, as resetting the field does.
fn chosen_limit(gib: f64) -> Option<f64> {
    (gib != DEFAULT_GIB).then_some(gib)
}

struct SavedLimit {
    store: Option<config::Store>,
    gib: Option<f64>,
    keeps_none: bool,
    error: Option<SharedString>,
}

/// The saved limit, and why it could not be read or will not be saved.
fn read_saved_limit() -> SavedLimit {
    let store = match config::Store::user() {
        Ok(store) => store,
        Err(error) => {
            return SavedLimit {
                store: None,
                gib: None,
                keeps_none: false,
                error: Some(format!("The song cache limit won't be saved: {error}.").into()),
            };
        }
    };
    match store.song_cache_gib() {
        Ok(gib) => SavedLimit {
            store: Some(store),
            gib,
            keeps_none: false,
            error: None,
        },
        Err(error) => SavedLimit {
            store: Some(store),
            gib: None,
            keeps_none: true,
            error: Some(
                format!("Couldn't read the song cache limit, so no songs are being kept: {error}.")
                    .into(),
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_limit_leaves_the_setting_unset() {
        assert_eq!(chosen_limit(DEFAULT_GIB), None);
        assert_eq!(chosen_limit(0.0), Some(0.0));
        assert_eq!(chosen_limit(0.5), Some(0.5));
    }
}
