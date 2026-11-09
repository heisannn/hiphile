use std::sync::{Arc, Weak};

use catalog::SongStore;
use executor::{Executor, Task};
use tokio::sync::watch;

use executor::{Publish, Published};

use super::Freshness;

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
    /// Which count is the latest. One under way on its thread when the songs go would report
    /// what was there.
    count_id: u64,
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
                count_id: 0,
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

    /// Counts now and again every two seconds for as long as it is held, so that songs kept
    /// meanwhile show up.
    pub fn keep_fresh(&self) -> Freshness {
        self.count();
        Freshness::keep(&self.0.executor, self.downgrade(), |this| {
            SongCacheUsage(this).count();
        })
    }

    fn start_count(&self, state: &mut State) {
        let Some(songs) = self.0.songs.clone() else {
            return;
        };
        if state.is_counting || state.usage.is_clearing {
            return;
        }
        state.is_counting = true;
        state.count_id += 1;
        let count_id = state.count_id;
        let this = self.downgrade();
        state._count = self.0.executor.spawn_blocking(move || {
            let kept = songs.kept_bytes();
            if let Some(this) = this.upgrade() {
                this.state.update(|state| {
                    if state.count_id == count_id {
                        state.is_counting = false;
                        state.usage.kept = Some(kept);
                    }
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
            state.is_counting = false;
            state.count_id += 1;
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
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::time::Duration;

    use executor::{BoxFuture, Detach, Spawned, Spawner};
    use gpui_kit::TestAppContext;

    use super::*;
    use crate::settings::freshness::REFRESH;

    type Work = Box<dyn FnOnce() + Send>;

    /// Runs blocking work when the test says, to its end even once it was dropped, as a thread
    /// that has started on it does.
    #[derive(Clone, Default)]
    struct Threads(Arc<Mutex<VecDeque<Work>>>);

    struct Started;

    impl Detach for Started {
        fn detach(self: Box<Self>) {}
    }

    impl Spawner for Threads {
        fn spawn(&self, _: BoxFuture<'static, ()>) -> Spawned {
            unreachable!("counting and removing spawn no future")
        }

        fn spawn_blocking(&self, work: Work) -> Spawned {
            self.0.lock().unwrap().push_back(work);
            Spawned::new(Started)
        }

        fn timer(&self, _: Duration) -> BoxFuture<'static, ()> {
            Box::pin(std::future::pending())
        }
    }

    impl Threads {
        fn run_next(&self) {
            let work = self.0.lock().unwrap().pop_front().expect("work to run");
            work();
        }
    }

    fn keeping(bytes: usize) -> (tempfile::TempDir, SongStore) {
        let cache = tempfile::tempdir().unwrap();
        let songs = SongStore::in_cache(cache.path(), catalog::SONG_CAPACITY);
        put(&songs, "so-1", bytes);
        (cache, songs)
    }

    fn put(songs: &SongStore, id: &str, bytes: usize) {
        let server = url::Url::parse(crate::testing::NOWHERE).unwrap();
        songs
            .put(&server, &subsonic::SongId::new(id), &vec![0; bytes])
            .unwrap();
    }

    #[test]
    fn a_count_under_way_when_the_songs_go_does_not_bring_them_back() {
        let (_cache, songs) = keeping(100);
        let threads = Threads::default();
        let usage = SongCacheUsage::new(Executor::new(threads.clone()), Some(songs));

        usage.clear();
        threads.run_next();
        assert_eq!(usage.snapshot().kept(), None);

        threads.run_next();
        assert!(!usage.snapshot().is_clearing());
        threads.run_next();
        assert_eq!(usage.snapshot().kept(), Some(0));
    }

    #[gpui_kit::test]
    fn songs_kept_meanwhile_show_up_while_it_is_kept_fresh(cx: &mut TestAppContext) {
        let (_cache, songs) = keeping(100);
        let executor = cx.update(|cx| gpui_host::executor(cx));
        let usage = SongCacheUsage::new(executor, Some(songs.clone()));
        let fresh = usage.keep_fresh();
        cx.run_until_parked();
        assert_eq!(usage.snapshot().kept(), Some(100));

        put(&songs, "so-2", 50);
        cx.executor().advance_clock(REFRESH);
        cx.run_until_parked();
        assert_eq!(usage.snapshot().kept(), Some(150));

        drop(fresh);
        put(&songs, "so-3", 50);
        cx.executor().advance_clock(REFRESH);
        cx.run_until_parked();
        assert_eq!(usage.snapshot().kept(), Some(150));
    }

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
