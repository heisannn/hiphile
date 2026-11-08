use std::sync::{Arc, Weak};
use std::time::Duration;

use executor::{Executor, Saver, Task};
use tokio::sync::watch;

use executor::{Publish, Published};

/// The field reports every keystroke, and on the way to `16` a limit of `1` could remove files.
const SETTLE: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheKind {
    Songs,
    Covers,
}

impl CacheKind {
    pub fn default_gib(self) -> f64 {
        let bytes = match self {
            Self::Songs => catalog::SONG_CAPACITY,
            Self::Covers => catalog::COVER_CAPACITY,
        };
        (bytes >> 30) as f64
    }

    fn noun(self) -> &'static str {
        match self {
            Self::Songs => "song",
            Self::Covers => "cover",
        }
    }

    fn read(self, store: &config::Store) -> Result<Option<f64>, String> {
        match self {
            Self::Songs => store.song_cache_gib(),
            Self::Covers => store.cover_cache_gib(),
        }
        .map_err(|error| error.to_string())
    }

    fn save(self, store: &config::Store, gib: Option<f64>) -> Result<(), String> {
        match self {
            Self::Songs => store.save_song_cache_gib(gib),
            Self::Covers => store.save_cover_cache_gib(gib),
        }
        .map_err(|error| error.to_string())
    }
}

/// How many GiB may be kept, where `0` means no limit.
#[derive(Clone, Debug, PartialEq)]
pub struct CacheLimitSnapshot {
    kind: CacheKind,
    /// What was last chosen. `None` means the default limit.
    gib: Option<f64>,
    /// The limit in effect, which `gib` becomes once it settles.
    applied: Option<f64>,
    /// No limit is in effect then, so `applied` means nothing.
    keeps_none: bool,
    error: Option<String>,
}

impl CacheLimitSnapshot {
    pub fn kind(&self) -> CacheKind {
        self.kind
    }

    /// What was last chosen, which may not have settled yet.
    pub fn gib(&self) -> f64 {
        self.gib.unwrap_or_else(|| self.kind.default_gib())
    }

    /// The bytes to keep, or `None` for the default limit.
    pub fn capacity(&self) -> Option<u64> {
        self.applied.and_then(config::cache_capacity_of)
    }

    /// Whether nothing is being kept, because the limit could not be read. [`Self::capacity`]
    /// means nothing then.
    pub fn is_keeping_none(&self) -> bool {
        self.keeps_none
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

#[derive(Clone)]
pub struct CacheLimit(Arc<Inner>);

struct Inner {
    executor: Executor,
    /// `None` without a home directory: the limit then lasts until quitting.
    store: Option<config::Store>,
    saver: Option<Saver<Option<f64>>>,
    state: Published<State>,
}

struct State {
    limit: CacheLimitSnapshot,
    _settle: Task<()>,
}

impl Publish for State {
    type Snapshot = CacheLimitSnapshot;

    fn snapshot(&self) -> &CacheLimitSnapshot {
        &self.limit
    }
}

impl CacheLimit {
    pub fn new(
        kind: CacheKind,
        executor: Executor,
        store: Result<config::Store, config::Error>,
    ) -> Self {
        let saved = read_saved_limit(kind, store);
        let limit = CacheLimitSnapshot {
            kind,
            gib: saved.gib,
            applied: saved.gib,
            keeps_none: saved.keeps_none,
            error: saved.error,
        };
        Self(Arc::new_cyclic(|this: &Weak<Inner>| Inner {
            saver: saved.store.clone().map(|store| {
                let this = this.clone();
                Saver::new(executor.clone(), move |gib| {
                    let saved = kind.save(&store, gib);
                    if let Some(this) = this.upgrade() {
                        this.state.update(|state| {
                            state.limit.error = saved.err().map(|error| {
                                let noun = kind.noun();
                                format!("Couldn't save the {noun} cache limit: {error}.")
                            });
                        });
                    }
                })
            }),
            executor,
            store: saved.store,
            state: Published::new(State {
                limit,
                _settle: Task::ready(()),
            }),
        }))
    }

    pub fn snapshot(&self) -> CacheLimitSnapshot {
        self.0.state.snapshot()
    }

    /// Sees each new snapshot. A borrow of it must not be held while calling this limit, which
    /// publishes under its own lock.
    pub fn subscribe(&self) -> watch::Receiver<CacheLimitSnapshot> {
        self.0.state.subscribe()
    }

    /// Takes effect once no other has come for a moment.
    pub fn set_gib(&self, gib: f64) {
        let this = Arc::downgrade(&self.0);
        let executor = self.0.executor.clone();
        self.0.state.update(|state| {
            state.limit.gib = chosen_limit(state.limit.kind, gib);
            state._settle = self.0.executor.spawn(async move {
                executor.timer(SETTLE).await;
                if let Some(this) = this.upgrade() {
                    CacheLimit(this).apply();
                }
            });
        });
    }

    /// Goes back to the limit in effect, dropping a change that has not settled, as when the
    /// field is emptied on the way to another number.
    pub fn revert(&self) {
        self.0.state.update(|state| {
            state._settle = Task::ready(());
            state.limit.gib = state.limit.applied;
        });
    }

    fn apply(&self) {
        self.0.state.update(|state| {
            let limit = &mut state.limit;
            if limit.gib == limit.applied && !limit.keeps_none {
                return;
            }
            limit.applied = limit.gib;
            limit.keeps_none = false;
            if self.0.store.is_some() {
                limit.error = None;
            }
            // Under the lock, so that the limits reach the saver in the order they were applied
            if let Some(saver) = &self.0.saver {
                saver.save(limit.applied);
            }
        });
    }

    /// Writes a limit still settling or waiting for its turn, which nothing would write once the
    /// app has quit. Blocks on the disk.
    pub fn save_before_quitting(&self) {
        let (Some(store), Some(saver)) = (&self.0.store, &self.0.saver) else {
            return;
        };
        let (kind, gib, applied) = self.0.state.read(|state| {
            let limit = &state.limit;
            (limit.kind, limit.gib, limit.applied)
        });
        let waiting = saver.take_waiting();
        if gib != applied {
            kind.save(store, gib).ok();
        } else if let Some(gib) = waiting {
            kind.save(store, gib).ok();
        }
    }
}

/// The default leaves the setting unset, as resetting the field does.
fn chosen_limit(kind: CacheKind, gib: f64) -> Option<f64> {
    (gib != kind.default_gib()).then_some(gib)
}

struct SavedLimit {
    store: Option<config::Store>,
    gib: Option<f64>,
    keeps_none: bool,
    error: Option<String>,
}

fn read_saved_limit(kind: CacheKind, store: Result<config::Store, config::Error>) -> SavedLimit {
    let noun = kind.noun();
    let store = match store {
        Ok(store) => store,
        Err(error) => {
            return SavedLimit {
                store: None,
                gib: None,
                keeps_none: false,
                error: Some(format!("The {noun} cache limit won't be saved: {error}.")),
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
            error: Some(format!(
                "Couldn't read the {noun} cache limit, so no more {noun}s are being kept: {error}."
            )),
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

    #[gpui_kit::test]
    fn a_limit_takes_effect_and_is_saved_once_it_settles(cx: &mut gpui_kit::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let executor = cx.update(|cx| gpui_host::executor(cx));
        let limit = CacheLimit::new(CacheKind::Songs, executor, Ok(store.clone()));

        limit.set_gib(1.0);
        limit.set_gib(16.0);
        cx.run_until_parked();
        assert_eq!(limit.snapshot().gib(), 16.0);
        assert_eq!(limit.snapshot().capacity(), None);

        cx.executor().advance_clock(SETTLE);
        cx.run_until_parked();
        assert_eq!(limit.snapshot().capacity(), Some(16 << 30));
        assert_eq!(store.song_cache_gib().unwrap(), Some(16.0));
    }

    #[gpui_kit::test]
    fn a_limit_still_settling_is_written_before_quitting(cx: &mut gpui_kit::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let executor = cx.update(|cx| gpui_host::executor(cx));
        let limit = CacheLimit::new(CacheKind::Covers, executor, Ok(store.clone()));

        limit.set_gib(2.0);
        limit.save_before_quitting();
        assert_eq!(store.cover_cache_gib().unwrap(), Some(2.0));
    }
}
