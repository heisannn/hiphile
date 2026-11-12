use std::sync::{Arc, Weak};

use executor::{Executor, Publish, Published, Task};
use fuzzy::{Index, Speller};
use session::Session;
use subsonic::Artist;
use tokio::sync::watch;

use super::list::{ArtistDates, ArtistOrder, ArtistSort, arrange, artist_index, artists_of_albums};
use crate::shown::{Arranged, Arranger, Arranging, is_waiting, take_reading};
use crate::{Shelf, Shown, follow};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtistListStatus {
    /// The artists read last, which show once they are indexed and in order.
    artists: Shelf<Artist>,
    shown: Option<Shown<Artist>>,
    /// From the saved albums: without a saved library, no artist has a date.
    dates: Arc<ArtistDates>,
    query: String,
    sort: ArtistSort,
    is_reading: bool,
    /// Refreshing artists already on show failed, for this reason. Artists that never showed
    /// fail as [`Shelf::Failed`] instead.
    failure: Option<String>,
}

impl ArtistListStatus {
    /// In the server's name order.
    pub fn artists(&self) -> &Shelf<Artist> {
        &self.artists
    }

    /// What shows, which stays while newer artists wait for their index and their order. `None`
    /// until the first artists have both.
    pub fn shown(&self) -> Option<&Shown<Artist>> {
        self.shown.as_ref()
    }

    pub fn dates(&self) -> &Arc<ArtistDates> {
        &self.dates
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn sort(&self) -> ArtistSort {
        self.sort
    }

    /// Whether artists are being read, or wait to show.
    pub fn is_refreshing(&self) -> bool {
        self.is_reading || is_waiting(&self.artists, self.shown.as_ref())
    }

    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }
}

/// The artists, as saved and as the server has them, arranged by a query and a sort. Nothing is
/// read until [`ArtistList::open`], since a list that may never show is not worth reading. A query
/// searches the artists as they read with the dictionaries when they are there, and they are read
/// again when the dictionaries come or go. Dropping the last handle stops a reading under way.
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
    arranging: Arranging<Artist>,
    _work: Task<()>,
    _readings_changes: Task<()>,
}

impl ArtistList {
    pub fn new(executor: Executor, session: Session) -> Self {
        let this = Self(Arc::new(Inner {
            executor,
            session,
            state: Published::new(State {
                status: ArtistListStatus {
                    artists: Shelf::Loading,
                    shown: None,
                    dates: Arc::default(),
                    query: String::new(),
                    sort: ArtistSort::default(),
                    is_reading: false,
                    failure: None,
                },
                is_open: false,
                work_id: 0,
                arranging: Arranging::default(),
                _work: Task::ready(()),
                _readings_changes: Task::ready(()),
            }),
        }));
        if let Some(readings) = this.0.session.readings() {
            let weak = this.downgrade();
            let changes = follow::on_dictionaries_changed(&this.0.executor, readings, move || {
                weak.update(|this, state| this.index(state));
            });
            this.0
                .state
                .update(|state| state._readings_changes = changes);
        }
        this
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

    pub fn set_query(&self, query: &str) {
        self.0.state.update(|state| {
            if state.status.query != query {
                state.status.query = query.to_owned();
                self.arrange(state);
            }
        });
    }

    /// Choosing the order already chosen keeps its direction.
    pub fn choose_order(&self, order: ArtistOrder) {
        self.0.state.update(|state| {
            if state.status.sort.order != order {
                state.status.sort = ArtistSort::chosen(order);
                self.arrange(state);
            }
        });
    }

    pub fn choose_direction(&self, is_descending: bool) {
        self.0.state.update(|state| {
            if state.status.sort.is_descending != is_descending {
                state.status.sort.is_descending = is_descending;
                self.arrange(state);
            }
        });
    }

    /// Shows the saved artists, which a sync keeps up to date. Before there are any, shows what
    /// the saved albums say, then what the server says.
    fn load(&self, state: &mut State) {
        state.status.is_reading = true;
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
                        this.change(work_id, |this, state| {
                            let has_new_dates = state.set_dates(dates);
                            if state.finish(Ok(artists)) {
                                this.index(state);
                            } else if has_new_dates {
                                this.arrange(state);
                            }
                        });
                        return;
                    }
                    Ok((artists, dates, false)) => this.change(work_id, |this, state| {
                        let has_new_dates = state.set_dates(dates);
                        if !state.status.artists.is_loaded() && !artists.is_empty() {
                            state.status.artists = Shelf::Loaded(artists.into());
                            this.index(state);
                        } else if has_new_dates {
                            this.arrange(state);
                        }
                    }),
                    Err(_) => {}
                }
            }
            let result = client
                .artists()
                .await
                .map_err(|error| session::describe(&error));
            this.change(work_id, |this, state| {
                if state.finish(result) {
                    this.index(state);
                }
            });
        });
    }

    fn index(&self, state: &mut State) {
        if let Shelf::Loaded(artists) = &state.status.artists {
            let artists = artists.clone();
            self.arranger().index(state, artists);
        }
    }

    fn arrange(&self, state: &mut State) {
        self.arranger().arrange(state);
    }

    fn arranger(&self) -> Arranger<State> {
        let this = self.downgrade();
        Arranger::new(
            self.0.executor.clone(),
            self.0.session.clone(),
            move |change| this.update(|_, state| change(state)),
        )
    }

    fn downgrade(&self) -> WeakList {
        WeakList(Arc::downgrade(&self.0))
    }
}

impl State {
    /// Whether other artists came, which need an index.
    fn finish(&mut self, result: Result<Vec<Artist>, String>) -> bool {
        self.status.is_reading = false;
        take_reading(&mut self.status.artists, &mut self.status.failure, result)
    }

    /// Whether the dates changed, which may change the order. The same dates keep their `Arc`.
    fn set_dates(&mut self, dates: Arc<ArtistDates>) -> bool {
        let has_changed = *self.status.dates != *dates;
        if has_changed {
            self.status.dates = dates;
        }
        has_changed
    }
}

impl Arranged for State {
    type Item = Artist;
    type Order = (String, Arc<ArtistDates>, ArtistSort);

    fn arranging(&mut self) -> &mut Arranging<Artist> {
        &mut self.arranging
    }

    fn shown(&mut self) -> &mut Option<Shown<Artist>> {
        &mut self.status.shown
    }

    fn order(&self) -> Self::Order {
        let status = &self.status;
        (status.query.clone(), status.dates.clone(), status.sort)
    }

    fn index(artists: &[Artist], speller: Option<&dyn Speller>) -> Index {
        artist_index(artists, speller)
    }

    fn arrange(artists: &[Artist], index: &Index, (query, dates, sort): Self::Order) -> Vec<usize> {
        arrange(artists, index.search(&query), &dates, sort)
    }
}

impl Publish for State {
    type Snapshot = ArtistListStatus;

    fn snapshot(&self) -> &ArtistListStatus {
        &self.status
    }
}

impl WeakList {
    fn update(&self, change: impl FnOnce(&ArtistList, &mut State)) {
        if let Some(this) = self.0.upgrade().map(ArtistList) {
            this.0.state.update(|state| change(&this, state));
        }
    }

    /// Changes the state unless the work `work_id` has been replaced.
    fn change(&self, work_id: u64, change: impl FnOnce(&ArtistList, &mut State)) {
        self.update(|this, state| {
            if state.work_id == work_id {
                change(this, state);
            }
        });
    }
}

#[cfg(test)]
mod tests;
