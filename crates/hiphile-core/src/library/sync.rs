use std::mem;
use std::pin::pin;
use std::sync::{Arc, Weak};

use catalog::{Mode, Outcome};
use executor::{Executor, Publish, Published, Task};
use futures::channel::oneshot;
use futures::future::{self, Either};
use session::Session;
use subsonic::Album;
use tokio::sync::{broadcast, watch};

const NOTICES: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncStage {
    Albums,
    Artists,
    Songs,
}

/// How far a sync that is fetching has got with its stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyncProgress {
    stage: SyncStage,
    saved: usize,
    total: Option<usize>,
}

impl SyncProgress {
    pub fn stage(&self) -> SyncStage {
        self.stage
    }

    pub fn saved(&self) -> usize {
        self.saved
    }

    /// About how many in all, when that can be told.
    pub fn total(&self) -> Option<usize> {
        self.total
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Albums {
    Loading,
    Loaded(Arc<[Album]>),
    /// They never showed, for this reason.
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncStatus {
    albums: Albums,
    is_running: bool,
    /// The last a sync under way said of its stages, until it ends.
    progress: Option<SyncProgress>,
    /// Updating albums already on show failed, for this reason. Albums that never showed fail as
    /// [`Albums::Failed`] instead.
    failure: Option<String>,
}

impl SyncStatus {
    pub fn albums(&self) -> &Albums {
        &self.albums
    }

    pub fn is_running(&self) -> bool {
        self.is_running
    }

    pub fn progress(&self) -> Option<SyncProgress> {
        self.progress
    }

    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LibraryNotice {
    AlbumsSynced,
    ArtistsSynced,
    SongsSynced,
    /// A sync ended, however that went, with no other to follow.
    SyncFinished,
    /// A sync the user asked for failed, for why.
    SyncFailed(String),
}

/// Keeps the saved library up to date with the server, and the albums it holds. The first sync
/// starts when it is made, showing the saved albums, then bringing them and every song up to date
/// if they are stale. Dropping the last handle stops a sync under way.
#[derive(Clone)]
pub struct LibrarySync(Arc<Inner>);

#[derive(Clone)]
struct WeakSync(Weak<Inner>);

struct Inner {
    executor: Executor,
    session: Session,
    state: Published<State>,
    notices: broadcast::Sender<LibraryNotice>,
}

struct State {
    status: SyncStatus,
    /// A refresh was asked for while a sync ran, to run once it ends rather than cut it short.
    again: bool,
    /// Whether the sync under way or last run is one the user asked for, whose failure they
    /// should hear of wherever they are.
    is_requested: bool,
    work_id: u64,
    _work: Task<()>,
}

impl LibrarySync {
    pub fn new(executor: Executor, session: Session) -> Self {
        let this = Self(Arc::new(Inner {
            executor,
            session,
            state: Published::new(State {
                status: SyncStatus {
                    albums: Albums::Loading,
                    is_running: false,
                    progress: None,
                    failure: None,
                },
                again: false,
                is_requested: false,
                work_id: 0,
                _work: Task::ready(()),
            }),
            notices: broadcast::Sender::new(NOTICES),
        }));
        this.0
            .state
            .update(|state| this.start(state, Mode::IfStale));
        this
    }

    pub fn status(&self) -> SyncStatus {
        self.0.state.snapshot()
    }

    /// Sees each new status. A borrow of it must not be held while calling the sync, which
    /// publishes under its own lock.
    pub fn subscribe(&self) -> watch::Receiver<SyncStatus> {
        self.0.state.subscribe()
    }

    /// Hears each notice from now on.
    pub fn notices(&self) -> broadcast::Receiver<LibraryNotice> {
        self.0.notices.subscribe()
    }

    /// Fetches the whole library from the server again.
    pub fn refresh(&self) {
        self.0.state.update(|state| self.start(state, Mode::Always));
    }

    fn start(&self, state: &mut State, mode: Mode) {
        if state.status.is_running {
            state.again |= mode == Mode::Always;
            return;
        }
        state.status.is_running = true;
        state.status.failure = None;
        state.is_requested = mode == Mode::Always;
        if !matches!(state.status.albums, Albums::Loaded(_)) {
            state.status.albums = Albums::Loading;
        }
        state.work_id += 1;
        let work_id = state.work_id;
        let this = self.downgrade();
        let executor = self.0.executor.clone();
        let session = self.0.session.clone();
        state._work = self.0.executor.spawn(async move {
            let result = run(&this, work_id, &executor, &session, mode).await;
            this.change(work_id, |this, state| this.finish(state, result));
        });
    }

    fn finish(&self, state: &mut State, result: Result<(), String>) {
        let again = mem::take(&mut state.again);
        if let (Err(error), true, false) = (&result, state.is_requested, again) {
            self.notify(LibraryNotice::SyncFailed(error.clone()));
        }
        state.status.is_running = false;
        match result {
            Ok(()) => {}
            Err(error) if matches!(state.status.albums, Albums::Loaded(_)) => {
                state.status.failure = Some(error);
            }
            Err(error) => state.status.albums = Albums::Failed(error),
        }
        if again {
            self.start(state, Mode::Always);
        } else {
            state.status.progress = None;
            self.notify(LibraryNotice::SyncFinished);
        }
    }

    fn notify(&self, notice: LibraryNotice) {
        // Fails only when nobody listens
        self.0.notices.send(notice).ok();
    }

    fn downgrade(&self) -> WeakSync {
        WeakSync(Arc::downgrade(&self.0))
    }
}

impl State {
    fn set_albums(&mut self, albums: Vec<Album>) {
        self.status.albums = Albums::Loaded(albums.into());
    }
}

impl Publish for State {
    type Snapshot = SyncStatus;

    fn snapshot(&self) -> &SyncStatus {
        &self.status
    }
}

impl WeakSync {
    fn upgrade(&self) -> Option<LibrarySync> {
        self.0.upgrade().map(LibrarySync)
    }

    /// Changes the state unless the work `work_id` has been replaced.
    fn change(&self, work_id: u64, change: impl FnOnce(&LibrarySync, &mut State)) {
        if let Some(this) = self.upgrade() {
            this.0.state.update(|state| {
                if state.work_id == work_id {
                    change(&this, state);
                }
            });
        }
    }

    fn has_albums(&self) -> bool {
        self.upgrade().is_some_and(|this| {
            this.0
                .state
                .read(|state| matches!(state.status.albums, Albums::Loaded(_)))
        })
    }
}

/// Brings the library up to date with the server, through the saved library when there is one.
/// Errors are sentences for the user.
async fn run(
    this: &WeakSync,
    work_id: u64,
    executor: &Executor,
    session: &Session,
    mode: Mode,
) -> Result<(), String> {
    let client = session.client().clone();
    let describe = |error: client::Error| session::describe(&error);
    let describe_catalog = |error: catalog::Error| session::describe_catalog(&error);
    let Some(catalog) = session.catalog().cloned() else {
        let albums = client.albums_by_name().await.map_err(describe)?;
        this.change(work_id, |this, state| {
            state.set_albums(albums);
            this.notify(LibraryNotice::AlbumsSynced);
            // Without a copy, the songs are the server's, which may have changed
            this.notify(LibraryNotice::SongsSynced);
        });
        return Ok(());
    };

    let saved = || {
        let catalog = catalog.clone();
        executor.spawn_blocking(move || catalog.albums())
    };
    let mut shown = this.has_albums();
    if !shown {
        // A copy that can't be read is replaced by the sync below
        if let Ok(albums) = saved().await
            && !albums.is_empty()
        {
            this.change(work_id, |_, state| state.set_albums(albums));
            shown = true;
        }
    }

    let sync = |stage| watched_sync(this, work_id, executor, stage, &catalog, &client, mode);
    let outcome = sync(SyncStage::Albums).await.map_err(describe_catalog)?;
    let albums = match outcome {
        Outcome::Synced { .. } | Outcome::Unsettled { .. } => {
            Some(saved().await.map_err(describe_catalog)?)
        }
        Outcome::UpToDate if !shown => Some(saved().await.map_err(describe_catalog)?),
        // The CLI may be filling an empty copy, so ask the server directly
        Outcome::Busy if !shown => Some(client.albums_by_name().await.map_err(describe)?),
        Outcome::UpToDate | Outcome::Busy => None,
    };
    if let Some(albums) = albums {
        this.change(work_id, |this, state| {
            state.set_albums(albums);
            if is_changed(outcome) {
                this.notify(LibraryNotice::AlbumsSynced);
            }
        });
    }

    // The songs do not depend on the artists, so the sync goes on to them and reports the
    // failure at the end
    let artists = sync(SyncStage::Artists).await.map_err(describe_catalog);
    if matches!(artists, Ok(outcome) if is_changed(outcome)) {
        this.change(work_id, |this, _| this.notify(LibraryNotice::ArtistsSynced));
    }

    let songs = sync(SyncStage::Songs).await.map_err(describe_catalog);
    if matches!(songs, Ok(outcome) if is_changed(outcome)) {
        this.change(work_id, |this, _| this.notify(LibraryNotice::SongsSynced));
    }
    artists.and(songs).map(|_| ())
}

fn is_changed(outcome: Outcome) -> bool {
    matches!(outcome, Outcome::Synced { .. } | Outcome::Unsettled { .. })
}

/// Syncs `stage`, passing on what it has got to as it goes.
async fn watched_sync(
    this: &WeakSync,
    work_id: u64,
    executor: &Executor,
    stage: SyncStage,
    catalog: &catalog::Catalog,
    client: &client::Client,
    mode: Mode,
) -> Result<Outcome, catalog::Error> {
    let report = {
        let this = this.clone();
        move |progress: catalog::SyncProgress| {
            this.change(work_id, |_, state| {
                state.status.progress = Some(SyncProgress {
                    stage,
                    saved: progress.saved,
                    total: progress.total,
                });
            });
        }
    };
    let (catalog, client) = (catalog.clone(), client.clone());
    on_blocking_thread(executor, async move {
        match stage {
            SyncStage::Albums => {
                catalog::sync_albums_watched(&catalog, &client, mode, report).await
            }
            SyncStage::Artists => {
                catalog::sync_artists_watched(&catalog, &client, mode, report).await
            }
            SyncStage::Songs => catalog::sync_songs_watched(&catalog, &client, mode, report).await,
        }
    })
    .await
}

/// Runs `work` on a blocking thread, as a sync blocks on the disk between its requests. It stops
/// at its next await once the future waiting on it is dropped, which a blocking thread would not
/// notice on its own.
async fn on_blocking_thread<R: Send + 'static>(
    executor: &Executor,
    work: impl Future<Output = R> + Send + 'static,
) -> R {
    let (_stop, stopped) = oneshot::channel::<()>();
    let done = executor.spawn_blocking(move || {
        futures::executor::block_on(async move {
            match future::select(pin!(work), stopped).await {
                Either::Left((result, _)) => Some(result),
                Either::Right(_) => None,
            }
        })
    });
    match done.await {
        Some(result) => result,
        // Only once `_stop` is dropped, and this with it
        None => future::pending().await,
    }
}

#[cfg(test)]
mod tests;
