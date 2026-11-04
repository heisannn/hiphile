use std::time::Duration;

use gpui_kit::{AppContext as _, Context, EventEmitter, SharedString, Subscription, Task};

/// The most the field accepts, far beyond any disk.
pub(crate) const MAX_GIB: f64 = (1u32 << 20) as f64;
/// How long the limit must stay unchanged before it applies. The field reports
/// every keystroke, and on the way to `16` a limit of `1` could remove files.
const SETTLE: Duration = Duration::from_millis(500);

/// What a limit is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CacheKind {
    Songs,
    Covers,
}

impl CacheKind {
    /// What the settings keep when they leave the limit unset.
    pub(crate) fn default_gib(self) -> f64 {
        let bytes = match self {
            Self::Songs => catalog::SONG_CAPACITY,
            Self::Covers => catalog::COVER_CAPACITY,
        };
        (bytes >> 30) as f64
    }

    /// What is kept, as a sentence calls it.
    fn noun(self) -> &'static str {
        match self {
            Self::Songs => "song",
            Self::Covers => "cover",
        }
    }

    /// What the field is called, for screen readers.
    pub(crate) fn field_name(self) -> &'static str {
        match self {
            Self::Songs => "Song cache limit",
            Self::Covers => "Cover cache limit",
        }
    }

    /// The saved limit, or why it could not be read.
    fn read(self, store: &config::Store) -> Result<Option<f64>, String> {
        match self {
            Self::Songs => store.song_cache_gib(),
            Self::Covers => store.cover_cache_gib(),
        }
        .map_err(|error| error.to_string())
    }

    /// Saves the limit, or says why it could not be.
    fn save(self, store: &config::Store, gib: Option<f64>) -> Result<(), String> {
        match self {
            Self::Songs => store.save_song_cache_gib(gib),
            Self::Covers => store.save_cover_cache_gib(gib),
        }
        .map_err(|error| error.to_string())
    }
}

/// The limit changed; read it from [`CacheLimit::capacity`].
pub struct CacheLimitChanged;

/// How many GiB of something may be kept on this computer, where `0` means no
/// limit. Saving happens off the main thread.
pub struct CacheLimit {
    kind: CacheKind,
    /// `None` without a home directory: the limit then lasts until quitting.
    store: Option<config::Store>,
    /// What the field last held as a number. `None` means the default limit.
    gib: Option<f64>,
    /// The limit in effect, which `gib` becomes once it settles.
    applied: Option<f64>,
    /// Whether nothing is being kept, because the limit could not be read.
    /// No limit is in effect then, so `applied` means nothing.
    keeps_none: bool,
    error: Option<SharedString>,
    _settle: Task<()>,
    unsaved: bool,
    saving: bool,
    _save: Task<()>,
    _quit: Subscription,
}

impl EventEmitter<CacheLimitChanged> for CacheLimit {}

impl CacheLimit {
    pub(crate) fn new(kind: CacheKind, cx: &mut Context<Self>) -> Self {
        let saved = read_saved_limit(kind);
        Self {
            kind,
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
        self.applied.and_then(config::cache_capacity_of)
    }

    pub(crate) fn kind(&self) -> CacheKind {
        self.kind
    }

    pub(crate) fn gib(&self) -> f64 {
        self.gib.unwrap_or_else(|| self.kind.default_gib())
    }

    pub(crate) fn error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }

    pub(crate) fn set_gib(&mut self, gib: f64, cx: &mut Context<Self>) {
        self.gib = chosen_limit(self.kind, gib);
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
        cx.emit(CacheLimitChanged);
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
        let kind = self.kind;
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
                    .background_spawn(async move { kind.save(&store, gib) })
                    .await;
                this.update(cx, |this, cx| {
                    this.error = saved.err().map(|error| {
                        let noun = kind.noun();
                        format!("Couldn't save the {noun} cache limit: {error}.").into()
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
            self.kind.save(store, self.gib).ok();
        }
    }
}

/// The limit for `gib` typed into the field. The default leaves the setting
/// unset, as resetting the field does.
fn chosen_limit(kind: CacheKind, gib: f64) -> Option<f64> {
    (gib != kind.default_gib()).then_some(gib)
}

struct SavedLimit {
    store: Option<config::Store>,
    gib: Option<f64>,
    keeps_none: bool,
    error: Option<SharedString>,
}

/// The saved limit, and why it could not be read or will not be saved.
fn read_saved_limit(kind: CacheKind) -> SavedLimit {
    let noun = kind.noun();
    let store = match config::Store::user() {
        Ok(store) => store,
        Err(error) => {
            return SavedLimit {
                store: None,
                gib: None,
                keeps_none: false,
                error: Some(format!("The {noun} cache limit won't be saved: {error}.").into()),
            };
        }
    };
    match kind.read(&store) {
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
                format!(
                    "Couldn't read the {noun} cache limit, so no more {noun}s are being kept: {error}."
                )
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
        for kind in [CacheKind::Songs, CacheKind::Covers] {
            assert_eq!(chosen_limit(kind, kind.default_gib()), None);
            assert_eq!(chosen_limit(kind, 0.0), Some(0.0));
            assert_eq!(chosen_limit(kind, 0.5), Some(0.5));
        }
    }

    #[test]
    fn covers_keep_eight_gib_unless_told_otherwise() {
        assert_eq!(CacheKind::Covers.default_gib(), 8.0);
    }

    #[test]
    fn each_kind_keeps_its_own_limit() {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        CacheKind::Covers.save(&store, Some(2.0)).unwrap();
        CacheKind::Songs.save(&store, Some(3.0)).unwrap();
        assert_eq!(CacheKind::Covers.read(&store).unwrap(), Some(2.0));
        assert_eq!(CacheKind::Songs.read(&store).unwrap(), Some(3.0));
    }
}
