use std::sync::{Arc, Weak};

use executor::{Executor, Publish, Published, Task};
use session::Session;
use subsonic::Artist;
use tokio::sync::watch;

use super::list::{ArtistDates, artists_of_albums};
use crate::Shelf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtistListStatus {
    artists: Shelf<Artist>,
    /// From the saved albums: without a saved library, no artist has a date.
    dates: Arc<ArtistDates>,
    is_refreshing: bool,
    /// Refreshing artists already on show failed, for this reason. Artists that never showed
    /// fail as [`Shelf::Failed`] instead.
    failure: Option<String>,
}

impl ArtistListStatus {
    /// In the server's name order.
    pub fn artists(&self) -> &Shelf<Artist> {
        &self.artists
    }

    pub fn dates(&self) -> &Arc<ArtistDates> {
        &self.dates
    }

    pub fn is_refreshing(&self) -> bool {
        self.is_refreshing
    }

    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }
}

/// The artists, as saved and as the server has them. Nothing is read until [`ArtistList::open`],
/// since a list that may never show is not worth reading. Dropping the last handle stops a
/// reading under way.
#[derive(Clone)]
pub struct ArtistList(Arc<Inner>);

#[derive(Clone)]
struct WeakList(Weak<Inner>);

struct Inner {
    executor: Executor,
    session: Session,
    state: Published<State>,
}

struct State {
    status: ArtistListStatus,
    is_open: bool,
    /// Counts the readings, so that one replaced while it was finishing on another thread cannot
    /// change the list after its successor has.
    work_id: u64,
    _work: Task<()>,
}

impl ArtistList {
    pub fn new(executor: Executor, session: Session) -> Self {
        Self(Arc::new(Inner {
            executor,
            session,
            state: Published::new(State {
                status: ArtistListStatus {
                    artists: Shelf::Loading,
                    dates: Arc::default(),
                    is_refreshing: false,
                    failure: None,
                },
                is_open: false,
                work_id: 0,
                _work: Task::ready(()),
            }),
        }))
    }

    pub fn status(&self) -> ArtistListStatus {
        self.0.state.snapshot()
    }

    /// Sees each new status. A borrow of it must not be held while calling the list, which
    /// publishes under its own lock.
    pub fn subscribe(&self) -> watch::Receiver<ArtistListStatus> {
        self.0.state.subscribe()
    }

    /// Reads the artists the first time.
    pub fn open(&self) {
        self.0.state.update(|state| {
            if !state.is_open {
                state.is_open = true;
                self.load(state);
            }
        });
    }

    /// Reads the artists again, unless the list was never opened.
    pub fn reload(&self) {
        self.0.state.update(|state| {
            if state.is_open {
                self.load(state);
            }
        });
    }

    /// Shows the saved artists, which a sync keeps up to date. Before there are any, shows what
    /// the saved albums say, then what the server says.
    fn load(&self, state: &mut State) {
        state.status.is_refreshing = true;
        state.status.failure = None;
        state.status.artists.begin();
        state.work_id += 1;
        let work_id = state.work_id;
        let client = self.0.session.client().clone();
        let catalog = self.0.session.catalog().cloned();
        let executor = self.0.executor.clone();
        let this = self.downgrade();
        state._work = self.0.executor.spawn(async move {
            if let Some(catalog) = catalog {
                let saved = executor
                    .spawn_blocking(move || {
                        // Albums that can't be read only leave the artists without dates
                        let albums = catalog.albums();
                        let dates = albums.as_deref().map(ArtistDates::of_albums);
                        let dates = Arc::new(dates.unwrap_or_default());
                        let artists = catalog.artists()?;
                        if !artists.is_empty() {
                            return Ok((artists, dates, true));
                        }
                        Ok::<_, catalog::Error>((artists_of_albums(&albums?), dates, false))
                    })
                    .await;
                // A copy that can't be read only means waiting for the server
                match saved {
                    Ok((artists, dates, true)) => {
                        this.change(work_id, |state| {
                            state.status.dates = dates;
                            state.finish(Ok(artists));
                        });
                        return;
                    }
                    Ok((artists, dates, false)) => this.change(work_id, |state| {
                        state.status.dates = dates;
                        if !state.status.artists.is_loaded() && !artists.is_empty() {
                            state.status.artists = Shelf::Loaded(artists.into());
                        }
                    }),
                    Err(_) => {}
                }
            }
            let result = client
                .artists()
                .await
                .map_err(|error| session::describe(&error));
            this.change(work_id, |state| state.finish(result));
        });
    }

    fn downgrade(&self) -> WeakList {
        WeakList(Arc::downgrade(&self.0))
    }
}

impl State {
    fn finish(&mut self, result: Result<Vec<Artist>, String>) {
        self.status.is_refreshing = false;
        match result {
            Ok(artists) => self.status.artists = Shelf::Loaded(artists.into()),
            Err(error) if self.status.artists.is_loaded() => self.status.failure = Some(error),
            Err(error) => self.status.artists = Shelf::Failed(error),
        }
    }
}

impl Publish for State {
    type Snapshot = ArtistListStatus;

    fn snapshot(&self) -> &ArtistListStatus {
        &self.status
    }
}

impl WeakList {
    /// Changes the state unless the work `work_id` has been replaced.
    fn change(&self, work_id: u64, change: impl FnOnce(&mut State)) {
        if let Some(this) = self.0.upgrade() {
            this.state.update(|state| {
                if state.work_id == work_id {
                    change(state);
                }
            });
        }
    }
}

#[cfg(test)]
mod tests;
