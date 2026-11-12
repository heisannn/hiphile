use std::sync::{Arc, Weak};

use executor::{Executor, Publish, Published, Task};
use fuzzy::{Index, Speller};
use session::Session;
use subsonic::Album;
use tokio::sync::watch;

use super::filter::album_index;
use super::order::{AlbumOrder, AlbumSort, arrange};
use super::sync::{LibrarySync, SyncStatus};
use crate::shown::{Arranged, Arranger, Arranging, is_waiting};
use crate::{Shelf, Shown, follow};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlbumListStatus {
    /// The albums the sync shows last, in the server's name order, which show here once they are
    /// indexed and in order.
    albums: Shelf<Album>,
    shown: Option<Shown<Album>>,
    sort: AlbumSort,
    is_syncing: bool,
    /// Updating albums already on show failed, for this reason. Albums that never showed fail as
    /// [`Shelf::Failed`] instead.
    failure: Option<String>,
}

impl AlbumListStatus {
    pub fn albums(&self) -> &Shelf<Album> {
        &self.albums
    }

    /// What shows, which stays while newer albums wait for their index and their order. `None`
    /// until the first albums have both.
    pub fn shown(&self) -> Option<&Shown<Album>> {
        self.shown.as_ref()
    }

    pub fn sort(&self) -> AlbumSort {
        self.sort
    }

    /// Whether a sync is under way, or albums it brought wait to show.
    pub fn is_updating(&self) -> bool {
        self.is_syncing || is_waiting(&self.albums, self.shown.as_ref())
    }

    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }
}

/// The albums a sync keeps, arranged by a query and a sort. A query searches the albums as they
/// read with the dictionaries when they are there, and they are read again when the dictionaries
/// come or go. Dropping the last handle stops following the sync.
#[derive(Clone)]
pub struct AlbumList(Arc<Inner>);

#[derive(Clone)]
struct WeakList(Weak<Inner>);

struct Inner {
    executor: Executor,
    session: Session,
    state: Published<State>,
}

struct State {
    status: AlbumListStatus,
    /// The sync's albums as it gave them last, by which another `Arc` of the same albums is told
    /// apart from other albums without comparing them each time the sync says how far it got.
    synced: Shelf<Album>,
    query: String,
    arranging: Arranging<Album>,
    _follows: Vec<Task<()>>,
}

impl AlbumList {
    /// Starts from the albums `sync` shows already.
    pub fn new(executor: Executor, session: Session, sync: &LibrarySync) -> Self {
        let this = Self(Arc::new(Inner {
            executor,
            session,
            state: Published::new(State {
                status: AlbumListStatus {
                    albums: Shelf::Loading,
                    shown: None,
                    sort: AlbumSort::default(),
                    is_syncing: false,
                    failure: None,
                },
                synced: Shelf::Loading,
                query: String::new(),
                arranging: Arranging::default(),
                _follows: Vec::new(),
            }),
        }));
        let changes = sync.subscribe();
        let status = changes.borrow().clone();
        this.0
            .state
            .update(|state| this.follow_sync(state, &status));
        let weak = this.downgrade();
        let mut follows = vec![follow::observe(
            &this.0.executor,
            changes,
            move |status: SyncStatus| weak.update(|this, state| this.follow_sync(state, &status)),
        )];
        if let Some(readings) = this.0.session.readings() {
            let weak = this.downgrade();
            follows.push(follow::on_dictionaries_changed(
                &this.0.executor,
                readings,
                move || weak.update(|this, state| this.index(state)),
            ));
        }
        this.0.state.update(|state| state._follows = follows);
        this
    }

    pub fn status(&self) -> AlbumListStatus {
        self.0.state.snapshot()
    }

    /// Sees each new status. A borrow of it must not be held while calling the list, which
    /// publishes under its own lock.
    pub fn subscribe(&self) -> watch::Receiver<AlbumListStatus> {
        self.0.state.subscribe()
    }

    /// What the albums are searched for, as last set.
    pub fn query(&self) -> String {
        self.0.state.read(|state| state.query.clone())
    }

    pub fn set_query(&self, query: &str) {
        self.0.state.update(|state| {
            if state.query != query {
                state.query = query.to_owned();
                self.arranger().arrange(state);
            }
        });
    }

    /// Choosing the order already chosen keeps its direction.
    pub fn choose_order(&self, order: AlbumOrder) {
        self.0.state.update(|state| {
            if state.status.sort.order != order {
                state.status.sort = AlbumSort::chosen(order);
                self.arranger().arrange(state);
            }
        });
    }

    pub fn choose_direction(&self, is_descending: bool) {
        self.0.state.update(|state| {
            if state.status.sort.is_descending != is_descending {
                state.status.sort.is_descending = is_descending;
                self.arranger().arrange(state);
            }
        });
    }

    fn follow_sync(&self, state: &mut State, status: &SyncStatus) {
        state.status.is_syncing = status.is_running();
        state.status.failure = status.failure().map(str::to_owned);
        if is_same(&state.synced, status.albums()) {
            return;
        }
        state.synced = status.albums().clone();
        if state.status.albums != state.synced {
            state.status.albums = state.synced.clone();
            self.index(state);
        }
    }

    fn index(&self, state: &mut State) {
        if let Shelf::Loaded(albums) = &state.status.albums {
            let albums = albums.clone();
            self.arranger().index(state, albums);
        }
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

/// The same albums, by their `Arc`.
fn is_same(a: &Shelf<Album>, b: &Shelf<Album>) -> bool {
    match (a, b) {
        (Shelf::Loaded(a), Shelf::Loaded(b)) => Arc::ptr_eq(a, b),
        (a, b) => a == b,
    }
}

impl Arranged for State {
    type Item = Album;
    type Order = (String, AlbumSort);

    fn arranging(&mut self) -> &mut Arranging<Album> {
        &mut self.arranging
    }

    fn shown(&mut self) -> &mut Option<Shown<Album>> {
        &mut self.status.shown
    }

    fn order(&self) -> Self::Order {
        (self.query.clone(), self.status.sort)
    }

    fn index(albums: &[Album], speller: Option<&dyn Speller>) -> Index {
        album_index(albums, speller)
    }

    fn arrange(albums: &[Album], index: &Index, (query, sort): Self::Order) -> Vec<usize> {
        arrange(albums, index.search(&query), sort)
    }
}

impl Publish for State {
    type Snapshot = AlbumListStatus;

    fn snapshot(&self) -> &AlbumListStatus {
        &self.status
    }
}

impl WeakList {
    fn update(&self, change: impl FnOnce(&AlbumList, &mut State)) {
        if let Some(this) = self.0.upgrade().map(AlbumList) {
            this.0.state.update(|state| change(&this, state));
        }
    }
}

#[cfg(test)]
mod tests;
