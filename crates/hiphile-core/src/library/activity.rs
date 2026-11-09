use std::sync::{Arc, Weak};
use std::time::Duration;

use executor::{Executor, Publish, Published, Task};
use session::{ReadingsState, ReadingsStatus, Session};
use tokio::sync::watch;

use super::sync::{LibraryNotice, LibrarySync, SyncStage, SyncStatus};
use crate::follow;

/// How long working out how the library reads may take before it is worth mentioning: a few new
/// texts take no time at all.
const SLOW_READING: Duration = Duration::from_secs(1);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivityStatus {
    label: &'static str,
    done: usize,
    /// How many in all, when that can be told. A guess, which `done` may pass.
    total: Option<usize>,
    is_percentage: bool,
}

impl ActivityStatus {
    fn of_sync(stage: SyncStage, saved: usize, total: Option<usize>) -> Self {
        let label = match stage {
            SyncStage::Albums => "Syncing albums",
            SyncStage::Artists => "Syncing artists",
            SyncStage::Songs => "Syncing songs",
        };
        Self {
            label,
            done: saved,
            total,
            is_percentage: false,
        }
    }

    fn of_reading(done: usize, total: usize) -> Self {
        Self {
            label: "Preparing romaji search",
            done,
            total: Some(total),
            is_percentage: false,
        }
    }

    fn of_download(state: &ReadingsState) -> Option<Self> {
        let ReadingsState::Downloading(fraction) = state else {
            return None;
        };
        Some(Self {
            label: "Downloading dictionary",
            done: (fraction * 100.).round() as usize,
            total: Some(100),
            is_percentage: true,
        })
    }

    pub fn label(&self) -> &'static str {
        self.label
    }

    /// `total` while it still holds.
    fn known_total(&self) -> Option<usize> {
        self.total.filter(|total| *total > 0 && self.done <= *total)
    }

    /// From 0 to 100, when how many in all can be told.
    pub fn percent(&self) -> Option<f32> {
        self.known_total()
            .map(|total| self.done as f32 / total as f32 * 100.)
    }

    pub fn count(&self) -> Option<String> {
        if self.is_percentage {
            return Some(format!("{}%", self.done));
        }
        match self.known_total() {
            Some(total) => Some(format!("{} of {total}", self.done)),
            None => (self.done > 0).then(|| self.done.to_string()),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Notice {
    is_shown: bool,
    /// The user closed it, which holds until the library next goes quiet.
    is_dismissed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NoticeChange {
    Show,
    Hide,
}

impl Notice {
    fn follow(&mut self, is_busy: bool) -> Option<NoticeChange> {
        if !is_busy {
            self.is_dismissed = false;
        }
        match (is_busy, self.is_shown) {
            (true, false) if !self.is_dismissed => {
                self.is_shown = true;
                Some(NoticeChange::Show)
            }
            (false, true) => {
                self.is_shown = false;
                Some(NoticeChange::Hide)
            }
            _ => None,
        }
    }

    /// The notice went away. If it was up by then, the user closed it: taking it down is marked
    /// first, in [`Self::follow`].
    fn closed(&mut self) {
        if self.is_shown {
            self.is_shown = false;
            self.is_dismissed = true;
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ActivitySnapshot {
    /// What the notice last showed, to keep showing as it leaves.
    shown: Option<ActivityStatus>,
    is_notice_shown: bool,
}

impl ActivitySnapshot {
    pub fn shown(&self) -> Option<&ActivityStatus> {
        self.shown.as_ref()
    }

    /// Whether a notice of the activity should be up. It goes up once the library gets busy and
    /// comes down once it is quiet, unless the user closed it meanwhile.
    pub fn is_notice_shown(&self) -> bool {
        self.is_notice_shown
    }
}

/// What the library is busy with: a sync, the dictionaries' download, and working out how the
/// library reads after each sync, so nothing has to wait for it later. Dropping the last handle
/// stops following them.
#[derive(Clone)]
pub struct LibraryActivity(Arc<Inner>);

#[derive(Clone)]
struct WeakActivity(Weak<Inner>);

struct Inner {
    executor: Executor,
    session: Session,
    state: Published<State>,
}

struct State {
    snapshot: ActivitySnapshot,
    sync: Option<ActivityStatus>,
    download: Option<ActivityStatus>,
    reading: Option<ActivityStatus>,
    is_reading: bool,
    /// Something changed during the reading, which is then read again.
    read_again: bool,
    /// Counts the readings, so that a slow timer cannot outlast its own.
    reading_id: u64,
    is_reading_slow: bool,
    notice: Notice,
    _read: Task<()>,
    _slow_reading: Task<()>,
    _follows: Vec<Task<()>>,
}

impl LibraryActivity {
    pub fn new(executor: Executor, session: Session, sync: &LibrarySync) -> Self {
        let this = Self(Arc::new(Inner {
            executor,
            session,
            state: Published::new(State {
                snapshot: ActivitySnapshot::default(),
                sync: None,
                download: None,
                reading: None,
                is_reading: false,
                read_again: false,
                reading_id: 0,
                is_reading_slow: false,
                notice: Notice::default(),
                _read: Task::ready(()),
                _slow_reading: Task::ready(()),
                _follows: Vec::new(),
            }),
        }));
        let follows = this.follow(sync);
        this.0.state.update(|state| state._follows = follows);
        this
    }

    pub fn status(&self) -> ActivitySnapshot {
        self.0.state.snapshot()
    }

    /// Sees each new status. A borrow of it must not be held while calling the activity, which
    /// publishes under its own lock.
    pub fn subscribe(&self) -> watch::Receiver<ActivitySnapshot> {
        self.0.state.subscribe()
    }

    /// The notice went away, which the user may have done.
    pub fn notice_closed(&self) {
        self.0.state.update(|state| {
            state.notice.closed();
            state.snapshot.is_notice_shown = state.notice.is_shown;
        });
    }

    /// Starts from what the sync and the download are doing already: either may have begun
    /// before the activity was made.
    fn follow(&self, sync: &LibrarySync) -> Vec<Task<()>> {
        let executor = &self.0.executor;
        let this = self.downgrade();
        let notices = sync.notices();
        let sync_changes = sync.subscribe();
        let progress = sync_activity(&sync_changes.borrow());
        let mut follows = vec![
            follow::observe(executor, sync_changes, {
                let this = this.clone();
                move |status: SyncStatus| {
                    let progress = sync_activity(&status);
                    this.update(|this, state| this.sync_progress(state, progress));
                }
            }),
            follow::subscribe(executor, notices, {
                let this = this.clone();
                move |notice| {
                    if notice == LibraryNotice::SyncFinished {
                        this.update(|this, state| this.sync_finished(state));
                    }
                }
            }),
        ];
        let mut download = None;
        if let Some(readings) = self.0.session.readings() {
            follows.push(follow::on_dictionaries_changed(executor, readings, {
                let this = this.clone();
                move || this.update(|this, state| this.read_ahead(state))
            }));
            let changes = readings.subscribe();
            download = ActivityStatus::of_download(changes.borrow().state());
            follows.push(follow::observe(
                executor,
                changes,
                move |status: ReadingsStatus| {
                    let download = ActivityStatus::of_download(status.state());
                    this.update(|this, state| this.download_progress(state, download));
                },
            ));
        }
        self.0.state.update(|state| {
            self.sync_progress(state, progress);
            self.download_progress(state, download);
        });
        follows
    }

    fn sync_progress(&self, state: &mut State, sync: Option<ActivityStatus>) {
        if state.sync != sync {
            state.sync = sync;
            self.changed(state);
        }
    }

    fn sync_finished(&self, state: &mut State) {
        state.sync = None;
        self.changed(state);
        self.read_ahead(state);
    }

    fn download_progress(&self, state: &mut State, download: Option<ActivityStatus>) {
        if state.download != download {
            state.download = download;
            self.changed(state);
        }
    }

    fn read_ahead(&self, state: &mut State) {
        if state.is_reading {
            state.read_again = true;
            return;
        }
        let Some(book) = self.0.session.reading_book() else {
            return;
        };
        state.is_reading = true;
        state.reading_id += 1;
        let reading_id = state.reading_id;
        let this = self.downgrade();
        let executor = self.0.executor.clone();
        state._read = self.0.executor.spawn(async move {
            let progress = this.clone();
            executor
                .spawn_blocking(move || {
                    book.read_saved_library(|done, total| {
                        progress.update(|this, state| {
                            this.reading_progress(state, reading_id, done, total);
                        });
                    });
                })
                .await;
            this.update(|this, state| this.finish_reading(state));
        });
    }

    fn reading_progress(&self, state: &mut State, reading_id: u64, done: usize, total: usize) {
        if state.reading.is_none() {
            let this = self.downgrade();
            let timer = self.0.executor.timer(SLOW_READING);
            state._slow_reading = self.0.executor.spawn(async move {
                timer.await;
                this.update(|this, state| {
                    if state.reading_id == reading_id && state.is_reading {
                        state.is_reading_slow = true;
                        this.changed(state);
                    }
                });
            });
        }
        state.reading = Some(ActivityStatus::of_reading(done, total));
        self.changed(state);
    }

    fn finish_reading(&self, state: &mut State) {
        state.is_reading = false;
        state.reading = None;
        state.is_reading_slow = false;
        state._slow_reading = Task::ready(());
        self.changed(state);
        if std::mem::take(&mut state.read_again) {
            self.read_ahead(state);
        }
    }

    fn changed(&self, state: &mut State) {
        if let Some(status) = state.status() {
            state.snapshot.shown = Some(status.clone());
        }
        let is_busy = state.status().is_some();
        state.notice.follow(is_busy);
        state.snapshot.is_notice_shown = state.notice.is_shown;
    }

    fn downgrade(&self) -> WeakActivity {
        WeakActivity(Arc::downgrade(&self.0))
    }
}

fn sync_activity(status: &SyncStatus) -> Option<ActivityStatus> {
    let progress = status.progress()?;
    Some(ActivityStatus::of_sync(
        progress.stage(),
        progress.saved(),
        progress.total(),
    ))
}

impl State {
    /// What to show, the sync first. A reading is left out until it proves slow.
    fn status(&self) -> Option<&ActivityStatus> {
        self.sync
            .as_ref()
            .or(self.download.as_ref())
            .or(self.reading.as_ref().filter(|_| self.is_reading_slow))
    }
}

impl Publish for State {
    type Snapshot = ActivitySnapshot;

    fn snapshot(&self) -> &ActivitySnapshot {
        &self.snapshot
    }
}

impl WeakActivity {
    fn update(&self, change: impl FnOnce(&LibraryActivity, &mut State)) {
        if let Some(this) = self.0.upgrade().map(LibraryActivity) {
            this.0.state.update(|state| change(&this, state));
        }
    }
}

#[cfg(test)]
mod tests;
