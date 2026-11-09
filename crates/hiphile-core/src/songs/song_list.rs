use std::sync::{Arc, Weak};

use executor::{Executor, Publish, Published, Task};
use fuzzy::Index;
use session::Session;
use subsonic::Song;
use tokio::sync::watch;

use super::arrange::{Column, Sort, arrange, song_index};
use crate::{Shelf, follow};

/// Songs and the order they show in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShownSongs {
    songs: Arc<[Song]>,
    visible: Arc<[usize]>,
}

impl ShownSongs {
    pub fn songs(&self) -> &Arc<[Song]> {
        &self.songs
    }

    /// Positions in [`ShownSongs::songs`] of those that match the query, in the order they show.
    pub fn visible(&self) -> &Arc<[usize]> {
        &self.visible
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SongListStatus {
    /// The songs read last, which show once they are indexed and arranged.
    songs: Shelf<Song>,
    shown: Option<ShownSongs>,
    is_loading: bool,
    /// Reading songs again failed, for this reason. Songs that never showed fail as
    /// [`Shelf::Failed`] instead.
    failure: Option<String>,
    sort: Sort,
    /// Whether a query ranks what it finds rather than following `sort`. So it does until a sort
    /// is chosen; the next query after an empty one does again.
    by_relevance: bool,
    query: String,
}

impl SongListStatus {
    pub fn songs(&self) -> &Shelf<Song> {
        &self.songs
    }

    /// What shows, which stays while newer songs wait for their index and their order. `None`
    /// until the first songs have both.
    pub fn shown(&self) -> Option<&ShownSongs> {
        self.shown.as_ref()
    }

    pub fn is_loading(&self) -> bool {
        self.is_loading
    }

    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    pub fn sort(&self) -> Sort {
        self.sort
    }

    /// Whether the songs show by how well they match the query, so that no sort applies.
    pub fn is_ranked(&self) -> bool {
        self.by_relevance && !self.query.trim().is_empty()
    }
}

/// Every song in the library, read from the saved library or, with none saved, from the server,
/// and arranged by a query and a sort. Nothing is read until [`SongList::open`], since reading
/// every song is too much for a list that may never show. A query searches the songs as they read
/// with the dictionaries when they are there, and they are read again when the dictionaries come
/// or go. Dropping the last handle stops what is under way.
#[derive(Clone)]
pub struct SongList(Arc<Inner>);

#[derive(Clone)]
struct WeakList(Weak<Inner>);

struct Inner {
    executor: Executor,
    session: Session,
    state: Published<State>,
}

struct State {
    status: SongListStatus,
    is_open: bool,
    /// The songs the last index was built for, with it.
    indexed: Option<(Arc<[Song]>, Arc<Index>)>,
    /// Count the readings, the indexes and the arrangements, so that one replaced while it was
    /// finishing on another thread cannot replace its successor's result.
    load_id: u64,
    index_id: u64,
    arrange_id: u64,
    _load: Task<()>,
    _index: Task<()>,
    _arrange: Task<()>,
    _readings_changes: Task<()>,
}

impl SongList {
    pub fn new(executor: Executor, session: Session) -> Self {
        let this = Self(Arc::new(Inner {
            executor,
            session,
            state: Published::new(State {
                status: SongListStatus {
                    songs: Shelf::Loading,
                    shown: None,
                    is_loading: false,
                    failure: None,
                    sort: Sort::DEFAULT,
                    by_relevance: true,
                    query: String::new(),
                },
                is_open: false,
                indexed: None,
                load_id: 0,
                index_id: 0,
                arrange_id: 0,
                _load: Task::ready(()),
                _index: Task::ready(()),
                _arrange: Task::ready(()),
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

    pub fn status(&self) -> SongListStatus {
        self.0.state.snapshot()
    }

    /// Sees each new status. A borrow of it must not be held while calling the list, which
    /// publishes under its own lock.
    pub fn subscribe(&self) -> watch::Receiver<SongListStatus> {
        self.0.state.subscribe()
    }

    /// Reads the songs the first time.
    pub fn open(&self) {
        self.0.state.update(|state| {
            if !state.is_open {
                state.is_open = true;
                self.load(state);
            }
        });
    }

    /// Reads the songs again, unless the list was never opened.
    pub fn reload(&self) {
        self.0.state.update(|state| {
            if state.is_open {
                self.load(state);
            }
        });
    }

    pub fn set_query(&self, query: &str) {
        self.0.state.update(|state| {
            if state.status.query.trim().is_empty() {
                state.status.by_relevance = true;
            }
            state.status.query = query.to_owned();
            self.arrange(state);
        });
    }

    /// Choosing the column already sorted by keeps its direction.
    pub fn choose_sort(&self, column: Column) {
        self.0.state.update(|state| {
            if state.status.is_ranked() || state.status.sort.column != column {
                state.status.sort = Sort::chosen(column);
            }
            state.status.by_relevance = false;
            self.arrange(state);
        });
    }

    pub fn choose_direction(&self, is_descending: bool) {
        self.0.state.update(|state| {
            state.status.sort.is_descending = is_descending;
            state.status.by_relevance = false;
            self.arrange(state);
        });
    }

    /// Sorts by `column`, as a click on its heading does: the column already sorted by turns
    /// around.
    pub fn sort_by(&self, column: Column) {
        self.0.state.update(|state| {
            // While the songs follow relevance no column shows as sorted, so it starts from the
            // top rather than turning its column around
            if state.status.is_ranked() {
                state.status.sort.column = column;
                state.status.sort.is_descending = false;
            } else {
                state.status.sort = state.status.sort.clicked(column);
            }
            state.status.by_relevance = false;
            self.arrange(state);
        });
    }

    /// Shows the saved songs; with none saved yet, as during the first sync, asks the server for
    /// them.
    fn load(&self, state: &mut State) {
        state.status.is_loading = true;
        state.status.failure = None;
        state.status.songs.begin();
        state.load_id += 1;
        let load_id = state.load_id;
        let client = self.0.session.client().clone();
        let catalog = self.0.session.catalog().cloned();
        let executor = self.0.executor.clone();
        let this = self.downgrade();
        state._load = self.0.executor.spawn(async move {
            // A copy that can't be read is the same as none
            let saved = match catalog {
                Some(catalog) => executor
                    .spawn_blocking(move || catalog.songs().ok())
                    .await
                    .filter(|songs| !songs.is_empty()),
                None => None,
            };
            let result = match saved {
                Some(songs) => Ok(songs),
                None => client
                    .all_songs()
                    .await
                    .map_err(|error| session::describe(&error)),
            };
            this.update(|this, state| {
                if state.load_id == load_id && state.finish_loading(result) {
                    this.index(state);
                }
            });
        });
    }

    /// Off the host's thread: reading a large library with the dictionaries takes a while. The
    /// songs on show stay until the new ones are indexed and arranged.
    fn index(&self, state: &mut State) {
        state.index_id += 1;
        let Shelf::Loaded(songs) = &state.status.songs else {
            return;
        };
        let songs = songs.clone();
        let index_id = state.index_id;
        let book = self.0.session.reading_book();
        let executor = self.0.executor.clone();
        let this = self.downgrade();
        state._index = self.0.executor.spawn(async move {
            let index = executor
                .spawn_blocking({
                    let songs = songs.clone();
                    move || match book {
                        Some(book) => book.read(|speller| song_index(&songs, Some(speller))),
                        None => song_index(&songs, None),
                    }
                })
                .await;
            this.update(|this, state| {
                if state.index_id == index_id {
                    state.indexed = Some((songs, Arc::new(index)));
                    this.arrange(state);
                }
            });
        });
    }

    /// Off the host's thread: a large library takes a moment to sort.
    fn arrange(&self, state: &mut State) {
        state.arrange_id += 1;
        let Some((songs, index)) = state.indexed.clone() else {
            return;
        };
        let arrange_id = state.arrange_id;
        let query = state.status.query.clone();
        let sort = state.status.sort;
        let by_relevance = state.status.by_relevance;
        let executor = self.0.executor.clone();
        let this = self.downgrade();
        state._arrange = self.0.executor.spawn(async move {
            let visible = executor
                .spawn_blocking({
                    let songs = songs.clone();
                    move || arrange(&songs, &index, &query, sort, by_relevance)
                })
                .await;
            this.update(|_, state| {
                if state.arrange_id == arrange_id {
                    state.status.shown = Some(ShownSongs {
                        songs,
                        visible: visible.into(),
                    });
                }
            });
        });
    }

    fn downgrade(&self) -> WeakList {
        WeakList(Arc::downgrade(&self.0))
    }
}

impl State {
    /// Whether other songs came, which need an index.
    fn finish_loading(&mut self, result: Result<Vec<Song>, String>) -> bool {
        self.status.is_loading = false;
        match result {
            // The same songs keep their index
            Ok(songs) if self.status.songs.is_loaded() && *self.status.songs.items() == *songs => {
                false
            }
            Ok(songs) => {
                self.status.songs = Shelf::Loaded(songs.into());
                true
            }
            // The songs on show stay there, with word of what went wrong
            Err(error) if self.status.songs.is_loaded() => {
                self.status.failure = Some(error);
                false
            }
            Err(error) => {
                self.status.songs = Shelf::Failed(error);
                false
            }
        }
    }
}

impl Publish for State {
    type Snapshot = SongListStatus;

    fn snapshot(&self) -> &SongListStatus {
        &self.status
    }
}

impl WeakList {
    fn update(&self, change: impl FnOnce(&SongList, &mut State)) {
        if let Some(this) = self.0.upgrade().map(SongList) {
            this.0.state.update(|state| change(&this, state));
        }
    }
}

#[cfg(test)]
mod tests;
