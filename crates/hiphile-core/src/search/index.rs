use std::mem;
use std::sync::{Arc, Weak};

use executor::{Executor, Publish, Published, Task};
use session::Session;
use tokio::sync::watch;

use super::results::{Results, SearchLibrary};
use crate::follow;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IndexState {
    /// Read on the first [`SearchIndex::open`], since a library that may never be searched is
    /// not worth reading.
    Unread,
    Read,
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchStatus {
    library: IndexState,
    is_reading: bool,
    /// What the last search found, in what was read before while a newer reading is under way.
    results: Arc<Results>,
}

impl SearchStatus {
    pub fn library(&self) -> &IndexState {
        &self.library
    }

    pub fn is_reading(&self) -> bool {
        self.is_reading
    }

    /// Whether searches wait for a reading, as there is nothing read to search meanwhile.
    pub fn is_waiting(&self) -> bool {
        self.is_reading && self.library != IndexState::Read
    }

    /// Why the library could not be read, unless it is being read again.
    pub fn failure(&self) -> Option<&str> {
        match &self.library {
            IndexState::Failed(error) if !self.is_waiting() => Some(error),
            _ => None,
        }
    }

    pub fn results(&self) -> &Arc<Results> {
        &self.results
    }
}

/// What a search finds anything in: the saved library, read with the dictionaries when they are
/// there, and read again when the library or the dictionaries change. Dropping the last handle
/// drops what was read.
#[derive(Clone)]
pub struct SearchIndex(Arc<Inner>);

#[derive(Clone)]
struct WeakIndex(Weak<Inner>);

struct Inner {
    executor: Executor,
    session: Session,
    state: Published<State>,
}

struct State {
    status: SearchStatus,
    /// Stays in use while a newer reading is under way.
    library: Option<Arc<SearchLibrary>>,
    is_reading_out_of_date: bool,
    query: String,
    /// Counts the searches, so that one replaced while it was finishing on another thread cannot
    /// replace its successor's results.
    search_id: u64,
    _read: Task<()>,
    _search: Task<()>,
    _readings_changes: Task<()>,
}

impl SearchIndex {
    pub fn new(executor: Executor, session: Session) -> Self {
        let this = Self(Arc::new(Inner {
            executor,
            session,
            state: Published::new(State {
                status: SearchStatus {
                    library: IndexState::Unread,
                    is_reading: false,
                    results: Arc::default(),
                },
                library: None,
                is_reading_out_of_date: false,
                query: String::new(),
                search_id: 0,
                _read: Task::ready(()),
                _search: Task::ready(()),
                _readings_changes: Task::ready(()),
            }),
        }));
        if let Some(readings) = this.0.session.readings() {
            let weak = this.downgrade();
            let changes = follow::on_dictionaries_changed(&this.0.executor, readings, move || {
                weak.update(|this, state| this.library_changed_in(state));
            });
            this.0
                .state
                .update(|state| state._readings_changes = changes);
        }
        this
    }

    pub fn status(&self) -> SearchStatus {
        self.0.state.snapshot()
    }

    /// Sees each new status. A borrow of it must not be held while calling the index, which
    /// publishes under its own lock.
    pub fn subscribe(&self) -> watch::Receiver<SearchStatus> {
        self.0.state.subscribe()
    }

    /// Readies the index for searching: reads the library the first time, and again after a
    /// reading that failed.
    pub fn open(&self) {
        self.0.state.update(|state| {
            if matches!(
                state.status.library,
                IndexState::Unread | IndexState::Failed(_)
            ) {
                self.read(state);
            }
        });
    }

    /// Reads the library again, unless it was never read. Searches in the meantime go through
    /// what was read before.
    pub fn library_changed(&self) {
        self.0.state.update(|state| self.library_changed_in(state));
    }

    /// Finds `query`, and finds it again in each new reading. A search made before the library is
    /// read runs once it is.
    pub fn search(&self, query: &str) {
        self.0
            .state
            .update(|state| self.start_search(state, query.to_owned()));
    }

    fn library_changed_in(&self, state: &mut State) {
        if state.status.library != IndexState::Unread {
            self.read(state);
        }
    }

    fn read(&self, state: &mut State) {
        if state.status.is_reading {
            state.is_reading_out_of_date = true;
            return;
        }
        let Some(catalog) = self.0.session.catalog().cloned() else {
            let error = "There's no library saved on this computer to search. Check that the disk has free space, then sign out and sign in again.".to_owned();
            state.status.library = IndexState::Failed(error);
            return;
        };
        state.status.is_reading = true;
        let book = self.0.session.reading_book();
        let this = self.downgrade();
        let executor = self.0.executor.clone();
        state._read = self.0.executor.spawn(async move {
            let read = executor
                .spawn_blocking(move || {
                    let albums = catalog.albums()?;
                    let songs = catalog.songs()?;
                    Ok::<_, catalog::Error>(match book {
                        Some(book) => {
                            book.read(|speller| SearchLibrary::new(albums, songs, Some(speller)))
                        }
                        None => SearchLibrary::new(albums, songs, None),
                    })
                })
                .await;
            this.update(|this, state| this.finish_reading(state, read));
        });
    }

    fn finish_reading(&self, state: &mut State, read: Result<SearchLibrary, catalog::Error>) {
        state.status.is_reading = false;
        match read {
            Ok(library) => {
                state.library = Some(Arc::new(library));
                state.status.library = IndexState::Read;
            }
            Err(error) => {
                state.library = None;
                state.status.results = Arc::default();
                state.status.library = IndexState::Failed(session::describe_catalog(&error));
            }
        }
        if mem::take(&mut state.is_reading_out_of_date) {
            self.read(state);
        }
        let query = state.query.clone();
        self.start_search(state, query);
    }

    fn start_search(&self, state: &mut State, query: String) {
        state.search_id += 1;
        state._search = Task::ready(());
        state.query = query;
        if state.query.trim().is_empty() {
            state.status.results = Arc::default();
            return;
        }
        let Some(library) = state.library.clone() else {
            return;
        };
        let search_id = state.search_id;
        let query = state.query.clone();
        let this = self.downgrade();
        let executor = self.0.executor.clone();
        state._search = self.0.executor.spawn(async move {
            let results = executor
                .spawn_blocking(move || library.search(&query))
                .await;
            this.update(|_, state| {
                if state.search_id == search_id {
                    state.status.results = Arc::new(results);
                }
            });
        });
    }

    fn downgrade(&self) -> WeakIndex {
        WeakIndex(Arc::downgrade(&self.0))
    }
}

impl Publish for State {
    type Snapshot = SearchStatus;

    fn snapshot(&self) -> &SearchStatus {
        &self.status
    }
}

impl WeakIndex {
    fn update(&self, change: impl FnOnce(&SearchIndex, &mut State)) {
        if let Some(this) = self.0.upgrade().map(SearchIndex) {
            this.0.state.update(|state| change(&this, state));
        }
    }
}

#[cfg(test)]
mod tests;
