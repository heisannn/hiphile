use std::collections::{HashSet, VecDeque};
use std::mem;
use std::sync::{Arc, Weak};

use executor::{Executor, Publish, Published, Task};
use subsonic::{StarTarget, Starred};
use tokio::sync::{broadcast, watch};

use super::{Favorite, FavoriteLists};

const NOTICES: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FavoritesLoad {
    Loading,
    Loaded,
    Failed(String),
}

#[derive(Clone, PartialEq, Eq)]
pub struct FavoritesSnapshot {
    load: FavoritesLoad,
    lists: FavoriteLists,
    /// `None` until the favorites have loaded.
    known: Option<Arc<HashSet<StarTarget>>>,
    is_reloading: bool,
    refresh_error: Option<String>,
}

impl FavoritesSnapshot {
    pub fn load(&self) -> &FavoritesLoad {
        &self.load
    }

    pub fn is_loaded(&self) -> bool {
        self.load == FavoritesLoad::Loaded
    }

    /// Empty until they have loaded.
    pub fn lists(&self) -> &FavoriteLists {
        &self.lists
    }

    /// What every other screen goes by. `None` until the favorites have loaded.
    pub fn known(&self) -> Option<&Arc<HashSet<StarTarget>>> {
        self.known.as_ref()
    }

    /// `None` until the favorites have loaded.
    pub fn is_favorite(&self, target: &StarTarget) -> Option<bool> {
        Some(self.known.as_ref()?.contains(target))
    }

    /// Whether they are being read, the first time or again.
    pub fn is_reloading(&self) -> bool {
        self.is_reloading
    }

    /// Why reading them again failed, while those loaded before stay on show.
    pub fn refresh_error(&self) -> Option<&str> {
        self.refresh_error.as_deref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FavoritesNotice {
    /// The favorites could not be read and nothing else has them, with why.
    LoadFailed(String),
    /// A change the server refused or never heard of, which has been taken back, with why.
    WriteFailed(String),
}

/// The user's favorites, on show at once when changed and on the server one change at a time
/// behind. Starts loading when made: the menus of every screen say which way their command goes
/// from these. Dropping the last handle stops what is under way.
#[derive(Clone)]
pub struct FavoritesState(Arc<Inner>);

struct WeakFavorites(Weak<Inner>);

struct Inner {
    executor: Executor,
    client: client::Client,
    state: Published<State>,
    notices: broadcast::Sender<FavoritesNotice>,
}

struct State {
    favorites: FavoritesSnapshot,
    writes: VecDeque<Write>,
    is_sending: bool,
    /// Counts the changes, so that what the server said before the latest of them, which it may
    /// not reflect, is not shown over it.
    edits: u64,
    /// Whether to read the favorites once the writes have gone, as a read that overlaps one
    /// cannot be trusted to show it.
    reload_after_writes: bool,
    /// Counts the reads, so that one replaced while it was finishing on another thread cannot
    /// change the state after its successor has.
    load_id: u64,
    _load: Task<()>,
    _write: Task<()>,
}

/// A change to the favorites on its way to the server, already on show.
struct Write {
    target: StarTarget,
    is_favorite: bool,
    undo: Undo,
}

/// What brings back how the favorites showed before a change.
enum Undo {
    Remove,
    Restore { favorite: Box<Favorite>, ix: usize },
}

impl Write {
    fn failure(&self) -> &'static str {
        if self.is_favorite {
            "Couldn't add to favorites."
        } else {
            "Couldn't remove from favorites."
        }
    }
}

impl Publish for State {
    type Snapshot = FavoritesSnapshot;

    fn snapshot(&self) -> &FavoritesSnapshot {
        &self.favorites
    }
}

impl State {
    fn is_writing(&self) -> bool {
        self.is_sending || !self.writes.is_empty()
    }

    fn set_lists(&mut self, lists: FavoriteLists) {
        self.favorites.lists = lists;
        self.lists_changed();
    }

    fn lists_changed(&mut self) {
        self.favorites.known = Some(Arc::new(self.favorites.lists.targets()));
    }

    /// Shows `target` as it was before a change, wherever later changes have put it since.
    fn undo(&mut self, target: &StarTarget, undo: Undo) {
        let lists = &mut self.favorites.lists;
        lists.remove(target);
        if let Undo::Restore { favorite, ix } = undo {
            lists.add(*favorite, ix);
        }
        self.lists_changed();
    }
}

impl FavoritesState {
    pub fn new(executor: Executor, client: client::Client) -> Self {
        let this = Self(Arc::new(Inner {
            executor,
            client,
            state: Published::new(State {
                favorites: FavoritesSnapshot {
                    load: FavoritesLoad::Loading,
                    lists: FavoriteLists::default(),
                    known: None,
                    is_reloading: false,
                    refresh_error: None,
                },
                writes: VecDeque::new(),
                is_sending: false,
                edits: 0,
                reload_after_writes: false,
                load_id: 0,
                _load: Task::ready(()),
                _write: Task::ready(()),
            }),
            notices: broadcast::Sender::new(NOTICES),
        }));
        this.0.state.update(|state| this.load(state));
        this
    }

    pub fn snapshot(&self) -> FavoritesSnapshot {
        self.0.state.snapshot()
    }

    /// Sees each new snapshot. A borrow of it must not be held while calling the favorites,
    /// which publish under their own lock.
    pub fn subscribe(&self) -> watch::Receiver<FavoritesSnapshot> {
        self.0.state.subscribe()
    }

    /// Hears each failure from now on.
    pub fn notices(&self) -> broadcast::Receiver<FavoritesNotice> {
        self.0.notices.subscribe()
    }

    /// Reads the favorites again: another device may have changed them. Changes still being
    /// sent read them once they have gone.
    pub fn refresh(&self) {
        self.0.state.update(|state| self.load(state));
    }

    /// Reads the favorites again if they never arrived.
    pub fn reload_if_failed(&self) {
        self.0.state.update(|state| {
            if matches!(state.favorites.load, FavoritesLoad::Failed(_)) {
                self.load(state);
            }
        });
    }

    /// Adds `favorite` to the favorites or takes it out, on show at once and on the server as
    /// soon as what came before it has gone. Does nothing until the favorites have loaded.
    pub fn set_favorite(&self, favorite: Favorite, is_favorite: bool) {
        self.0
            .state
            .update(|state| self.set(state, favorite, is_favorite));
    }

    /// Takes `favorite` out of the favorites when it is one, as they stand with every change
    /// made so far, and adds it otherwise, as [`Self::set_favorite`] does.
    pub fn toggle(&self, favorite: Favorite) {
        self.0.state.update(|state| {
            let is_favorite = !state.favorites.lists.contains(&favorite.target());
            self.set(state, favorite, is_favorite);
        });
    }

    fn set(&self, state: &mut State, favorite: Favorite, is_favorite: bool) {
        let target = favorite.target();
        if !state.favorites.is_loaded() || state.favorites.lists.contains(&target) == is_favorite {
            return;
        }
        let lists = &mut state.favorites.lists;
        let undo = if is_favorite {
            lists.add(favorite, 0);
            Undo::Remove
        } else {
            let Some((favorite, ix)) = lists.remove(&target) else {
                return;
            };
            Undo::Restore {
                favorite: Box::new(favorite),
                ix,
            }
        };
        state.lists_changed();
        state.edits += 1;
        state.writes.push_back(Write {
            target,
            is_favorite,
            undo,
        });
        self.send_next_write(state);
    }

    fn load(&self, state: &mut State) {
        if state.is_writing() {
            state.reload_after_writes = true;
            return;
        }
        let favorites = &mut state.favorites;
        favorites.is_reloading = true;
        favorites.refresh_error = None;
        if !favorites.is_loaded() {
            favorites.load = FavoritesLoad::Loading;
        }
        state.load_id += 1;
        let (load_id, edits) = (state.load_id, state.edits);
        let this = self.downgrade();
        let client = self.0.client.clone();
        state._load = self.0.executor.spawn(async move {
            let result = client
                .starred()
                .await
                .map_err(|error| session::describe(&error));
            if let Some(this) = this.upgrade() {
                this.0.state.update(|state| {
                    this.finish_loading(state, load_id, edits, result);
                });
            }
        });
    }

    fn finish_loading(
        &self,
        state: &mut State,
        load_id: u64,
        edits: u64,
        result: Result<Starred, String>,
    ) {
        if state.load_id != load_id {
            return;
        }
        state.favorites.is_reloading = false;
        if state.edits != edits {
            self.load(state);
            return;
        }
        match result {
            Ok(starred) => {
                state.favorites.load = FavoritesLoad::Loaded;
                state.set_lists(FavoriteLists::new(starred));
            }
            // The favorites on show stay there, with word of what went wrong
            Err(error) if state.favorites.is_loaded() => {
                state.favorites.refresh_error = Some(error);
            }
            Err(error) => {
                state.favorites.load = FavoritesLoad::Failed(error.clone());
                self.tell(FavoritesNotice::LoadFailed(error));
            }
        }
    }

    /// Sends what waits, one at a time and in order: a later change overtaking an earlier one
    /// on the network would leave the older on the server.
    fn send_next_write(&self, state: &mut State) {
        if state.is_sending {
            return;
        }
        let Some(write) = state.writes.pop_front() else {
            if mem::take(&mut state.reload_after_writes) {
                self.load(state);
            }
            return;
        };
        state.is_sending = true;
        let this = self.downgrade();
        let client = self.0.client.clone();
        state._write = self.0.executor.spawn(async move {
            let result = if write.is_favorite {
                client.star(&write.target).await
            } else {
                client.unstar(&write.target).await
            };
            if let Some(this) = this.upgrade() {
                this.0
                    .state
                    .update(|state| this.finish_write(state, write, result));
            }
        });
    }

    fn finish_write(&self, state: &mut State, write: Write, result: Result<(), client::Error>) {
        state.is_sending = false;
        if let Err(error) = result {
            let message = format!("{} {}", write.failure(), session::describe(&error));
            self.tell(FavoritesNotice::WriteFailed(message));
            // The change may have got there with only the answer lost, which reading the
            // favorites back tells.
            state.reload_after_writes = true;
            let later = state
                .writes
                .iter_mut()
                .find(|later| later.target == write.target);
            if let Some(later) = later {
                // It decides what shows. Should it fail too, what it must go back to is what
                // this one started from.
                later.undo = write.undo;
            } else {
                state.undo(&write.target, write.undo);
            }
        }
        self.send_next_write(state);
    }

    fn tell(&self, notice: FavoritesNotice) {
        // Fails only when nobody listens
        self.0.notices.send(notice).ok();
    }

    fn downgrade(&self) -> WeakFavorites {
        WeakFavorites(Arc::downgrade(&self.0))
    }
}

impl WeakFavorites {
    fn upgrade(&self) -> Option<FavoritesState> {
        self.0.upgrade().map(FavoritesState)
    }
}

#[cfg(test)]
mod tests;
