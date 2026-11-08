use std::sync::{Arc, Weak};

use catalog::SongStore;
use executor::{Executor, Task};
use tokio::sync::watch;

use executor::{Publish, Published};

const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SongCacheSnapshot {
    /// `None` until first counted.
    kept: Option<u64>,
    is_clearing: bool,
    error: Option<String>,
}

impl SongCacheSnapshot {
    /// The bytes kept, or `None` until first counted.
    pub fn kept(&self) -> Option<u64> {
        self.kept
    }

    pub fn is_clearing(&self) -> bool {
        self.is_clearing
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

/// How much the song cache keeps, and removing all of it. Counting and removing touch every file,
/// so they run as blocking work.
#[derive(Clone)]
pub struct SongCacheUsage(Arc<Inner>);

struct Inner {
    executor: Executor,
    /// `None` where the system has no cache directory.
    songs: Option<SongStore>,
    state: Published<State>,
}

struct State {
    usage: SongCacheSnapshot,
    is_counting: bool,
    _count: Task<()>,
    _clear: Task<()>,
}

impl Publish for State {
    type Snapshot = SongCacheSnapshot;

    fn snapshot(&self) -> &SongCacheSnapshot {
        &self.usage
    }
}

impl SongCacheUsage {
    /// Counts at once. Counting and removing ignore the capacity that `songs` keeps to.
    pub fn new(executor: Executor, songs: Option<SongStore>) -> Self {
        let this = Self(Arc::new(Inner {
            executor,
            songs,
            state: Published::new(State {
                usage: SongCacheSnapshot::default(),
                is_counting: false,
                _count: Task::ready(()),
                _clear: Task::ready(()),
            }),
        }));
        this.count();
        this
    }

    pub fn snapshot(&self) -> SongCacheSnapshot {
        self.0.state.snapshot()
    }

    /// Sees each new snapshot. A borrow of it must not be held while calling this, which
    /// publishes under its own lock.
    pub fn subscribe(&self) -> watch::Receiver<SongCacheSnapshot> {
        self.0.state.subscribe()
    }

    /// Counts again, unless a count or a removal is under way.
    pub fn count(&self) {
        self.0.state.update(|state| self.start_count(state));
    }

    fn start_count(&self, state: &mut State) {
        let Some(songs) = self.0.songs.clone() else {
            return;
        };
        if state.is_counting || state.usage.is_clearing {
            return;
        }
        state.is_counting = true;
        let this = self.downgrade();
        state._count = self.0.executor.spawn_blocking(move || {
            let kept = songs.kept_bytes();
            if let Some(this) = this.upgrade() {
                this.state.update(|state| {
                    state.is_counting = false;
                    state.usage.kept = Some(kept);
                });
            }
        });
    }

    /// Removes every song kept for every server. A song being played goes on from its open file.
    pub fn clear(&self) {
        let Some(songs) = self.0.songs.clone() else {
            return;
        };
        self.0.state.update(|state| {
            if state.usage.is_clearing {
                return;
            }
            // A count in progress would report files that are going
            state.is_counting = false;
            state._count = Task::ready(());
            state.usage.is_clearing = true;
            state.usage.error = None;
            let this = self.downgrade();
            state._clear = self.0.executor.spawn_blocking(move || {
                let cleared = songs.clear();
                if let Some(this) = this.upgrade() {
                    let this = SongCacheUsage(this);
                    this.0.state.update(|state| {
                        state.usage.is_clearing = false;
                        state.usage.error = cleared
                            .err()
                            .map(|error| format!("Couldn't remove every song: {error}."));
                        this.start_count(state);
                    });
                }
            });
        });
    }

    fn downgrade(&self) -> Weak<Inner> {
        Arc::downgrade(&self.0)
    }
}

pub fn format_size(bytes: u64) -> String {
    let mib = (bytes + MIB / 2) / MIB;
    if mib < GIB / MIB {
        return format!("{mib} MiB");
    }
    let tenths = (bytes as f64 / GIB as f64 * 10.0).round() / 10.0;
    format!("{tenths} GiB")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_in_mib_then_gib() {
        assert_eq!(format_size(0), "0 MiB");
        assert_eq!(format_size(512 * MIB), "512 MiB");
        assert_eq!(format_size(GIB - 1), "1 GiB");
        assert_eq!(format_size(GIB), "1 GiB");
        assert_eq!(format_size(GIB + GIB / 2), "1.5 GiB");
        assert_eq!(format_size(8 * GIB), "8 GiB");
    }
}
