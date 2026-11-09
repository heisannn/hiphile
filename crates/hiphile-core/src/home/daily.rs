use std::sync::{Arc, Weak};
use std::time::Duration;

use executor::{Executor, Publish, Published, Task};
use session::Session;
use subsonic::{Album, AlbumId};
use tokio::sync::{broadcast, watch};

use super::recommend::{Recommendations, fetch_recommended, today};
use super::shelves::{MOST_PLAYED, NEW_ALBUMS};
use crate::Shelf;

/// A pick takes the server's outside service several seconds per album, and windows can come to
/// the front often.
const RETRY_AFTER: Duration = Duration::from_secs(10 * 60);
const NOTICES: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecommendedStatus {
    albums: Shelf<Album>,
    /// Whether the kept albums are being read or new ones picked.
    is_recommending: bool,
}

impl RecommendedStatus {
    /// The albums to try that no other shelf of the home screen shows, also when an album gets
    /// there after the day's pick. They stay on show while a pick that replaces them is on its
    /// way or fails.
    pub fn albums(&self) -> &Shelf<Album> {
        &self.albums
    }

    pub fn is_recommending(&self) -> bool {
        self.is_recommending
    }

    /// Whether albums show that a reload would replace. Until then their shelf shows the wait or
    /// offers to try again.
    pub fn is_reloadable(&self) -> bool {
        !self.albums.items().is_empty()
    }
}

/// Sent when a pick the user asked for went wrong and left the albums on show, which look the
/// same, for why.
#[derive(Clone, Debug)]
pub struct ReloadFailed {
    message: String,
}

impl ReloadFailed {
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// The albums to try. They stay the same all day, unless asked to reload, so they are kept in
/// the catalog and picked again on the first occasion of another day. Dropping the last handle
/// stops a pick under way.
#[derive(Clone)]
pub struct DailyRecommendations(Arc<Inner>);

#[derive(Clone)]
struct WeakRecommendations(Weak<Inner>);

struct Inner {
    executor: Executor,
    session: Session,
    state: Published<State>,
    notices: broadcast::Sender<ReloadFailed>,
}

struct State {
    status: RecommendedStatus,
    /// The day the albums on show were picked for.
    day: Option<String>,
    /// A pick went wrong a while ago, and is not made again by itself until this is over.
    is_waiting_to_retry: bool,
    /// What the other shelves show.
    shelved: Vec<AlbumId>,
    /// Counts the readings and picks, and the waits, so that one replaced while it was finishing
    /// on another thread cannot change the state after its successor has.
    work_id: u64,
    retry_id: u64,
    _work: Task<()>,
    _retry: Task<()>,
}

impl DailyRecommendations {
    /// Shows the albums kept from the last pick, whichever day that was, and picks new ones if it
    /// was not today.
    pub fn new(executor: Executor, session: Session) -> Self {
        let this = Self(Arc::new(Inner {
            executor,
            session,
            state: Published::new(State {
                status: RecommendedStatus {
                    albums: Shelf::Loading,
                    is_recommending: false,
                },
                day: None,
                is_waiting_to_retry: false,
                shelved: Vec::new(),
                work_id: 0,
                retry_id: 0,
                _work: Task::ready(()),
                _retry: Task::ready(()),
            }),
            notices: broadcast::Sender::new(NOTICES),
        }));
        this.0.state.update(|state| this.load(state));
        this
    }

    pub fn status(&self) -> RecommendedStatus {
        self.0.state.snapshot()
    }

    /// Sees each new status. A borrow of it must not be held while calling the recommendations,
    /// which publish under their own lock.
    pub fn subscribe(&self) -> watch::Receiver<RecommendedStatus> {
        self.0.state.subscribe()
    }

    /// Hears each failure of a reload from now on.
    pub fn notices(&self) -> broadcast::Receiver<ReloadFailed> {
        self.0.notices.subscribe()
    }

    /// Picks new albums if those on show are from another day. Call when something happens, for
    /// an app left open overnight.
    pub fn renew(&self) {
        self.0.state.update(|state| self.renew_in(state));
    }

    /// Picks new albums now, whichever day the ones on show are from.
    pub fn pick(&self) {
        self.0.state.update(|state| self.start_pick(state, false));
    }

    /// Picks new albums now, whichever day the ones on show are from. A pick that leaves them on
    /// show sends [`ReloadFailed`], as the albums look the same.
    pub fn reload(&self) {
        self.0.state.update(|state| self.start_pick(state, true));
    }

    /// A library sync may give a library that had nothing to pick from something to recommend,
    /// or other shelves may show every album picked.
    pub fn library_synced(&self) {
        self.0.state.update(|state| {
            if state.status.albums.items().is_empty() {
                state.day = None;
            }
            self.renew_in(state);
        });
    }

    fn load(&self, state: &mut State) {
        state.status.is_recommending = true;
        let work_id = state.next_work();
        let catalog = self.0.session.catalog().cloned();
        let user = self.0.session.username().to_owned();
        let this = self.downgrade();
        let executor = self.0.executor.clone();
        state._work = self.0.executor.spawn(async move {
            let kept = match catalog {
                Some(catalog) => executor
                    .spawn_blocking(move || catalog.recommended(&user))
                    .await
                    // The albums can be picked again
                    .ok()
                    .flatten(),
                None => None,
            };
            this.change(work_id, |this, state| {
                state.status.is_recommending = false;
                if let Some(kept) = kept {
                    state.day = Some(kept.day);
                    state.status.albums.finish(Ok(kept.albums));
                    state.keep_shelved_off();
                }
                this.renew_in(state);
            });
        });
    }

    fn renew_in(&self, state: &mut State) {
        if !state.status.is_recommending
            && !state.is_waiting_to_retry
            && state.day.as_deref() != Some(today().as_str())
        {
            self.start_pick(state, false);
        }
    }

    fn start_pick(&self, state: &mut State, is_asked: bool) {
        state.status.is_recommending = true;
        state.is_waiting_to_retry = false;
        state._retry = Task::ready(());
        state.status.albums.begin();
        let work_id = state.next_work();
        let client = self.0.session.client().clone();
        let day = today();
        let this = self.downgrade();
        state._work = self.0.executor.spawn(async move {
            let result = fetch_recommended(&client, NEW_ALBUMS, MOST_PLAYED)
                .await
                .map_err(|error| session::describe(&error));
            this.change(work_id, |this, state| {
                let kept = this.finish(state, result, day);
                if let (true, Some(message)) = (is_asked, kept) {
                    // Fails only when nobody listens
                    this.0.notices.send(ReloadFailed { message }).ok();
                }
            });
        });
    }

    /// Takes in a pick made for `day`. Only a complete one settles the day; the next occasion
    /// tries again after one that went wrong. Albums on show stay there unless a complete pick
    /// found others. Returns why, when something went wrong and they did.
    fn finish(
        &self,
        state: &mut State,
        result: Result<Recommendations, String>,
        day: String,
    ) -> Option<String> {
        state.status.is_recommending = false;
        let kept = (!state.status.albums.items().is_empty())
            .then(|| match &result {
                Ok(found) if found.complete => None,
                Ok(_) => Some("Couldn't get new recommendations. Try again later.".to_owned()),
                Err(error) => Some(format!("Couldn't get new recommendations. {error}")),
            })
            .flatten();
        match result {
            Ok(found) => {
                if found.complete {
                    if !found.albums.is_empty() {
                        self.keep(&day, &found.albums);
                    }
                    state.day = Some(day);
                } else {
                    self.wait_to_retry(state);
                }
                let better = found.complete && !found.albums.is_empty();
                if better || state.status.albums.items().is_empty() {
                    state.status.albums.finish(Ok(found.albums));
                    state.keep_shelved_off();
                }
            }
            Err(error) => {
                self.wait_to_retry(state);
                state.status.albums.finish(Err(error));
            }
        }
        kept
    }

    fn keep(&self, day: &str, albums: &[Album]) {
        let Some(catalog) = self.0.session.catalog().cloned() else {
            return;
        };
        let user = self.0.session.username().to_owned();
        let (day, albums) = (day.to_owned(), albums.to_vec());
        // Failing to keep them only means another pick next time
        self.0
            .executor
            .spawn_blocking(move || catalog.put_recommended(&user, &day, &albums).ok())
            .detach();
    }

    fn wait_to_retry(&self, state: &mut State) {
        state.is_waiting_to_retry = true;
        state.retry_id += 1;
        let retry_id = state.retry_id;
        let this = self.downgrade();
        let timer = self.0.executor.timer(RETRY_AFTER);
        state._retry = self.0.executor.spawn(async move {
            timer.await;
            if let Some(this) = this.upgrade() {
                this.0.state.update(|state| {
                    if state.retry_id == retry_id {
                        state.is_waiting_to_retry = false;
                    }
                });
            }
        });
    }

    /// Leaves out `shelved`, which the other shelves show, from the albums on show and from
    /// those picked later.
    pub(crate) fn keep_off(&self, shelved: Vec<AlbumId>) {
        self.0.state.update(|state| {
            state.shelved = shelved;
            state.keep_shelved_off();
        });
    }

    fn downgrade(&self) -> WeakRecommendations {
        WeakRecommendations(Arc::downgrade(&self.0))
    }
}

impl State {
    fn next_work(&mut self) -> u64 {
        self.work_id += 1;
        self.work_id
    }

    fn keep_shelved_off(&mut self) {
        let is_shelved = |album: &Album| self.shelved.contains(album.id());
        if let Shelf::Loaded(albums) = &self.status.albums
            && albums.iter().any(is_shelved)
        {
            let others = albums.iter().filter(|album| !is_shelved(album)).cloned();
            self.status.albums = Shelf::Loaded(others.collect());
        }
    }
}

impl Publish for State {
    type Snapshot = RecommendedStatus;

    fn snapshot(&self) -> &RecommendedStatus {
        &self.status
    }
}

impl WeakRecommendations {
    fn upgrade(&self) -> Option<DailyRecommendations> {
        self.0.upgrade().map(DailyRecommendations)
    }

    /// Changes the state unless the work `work_id` has been replaced.
    fn change(&self, work_id: u64, change: impl FnOnce(&DailyRecommendations, &mut State)) {
        if let Some(this) = self.upgrade() {
            this.0.state.update(|state| {
                if state.work_id == work_id {
                    change(&this, state);
                }
            });
        }
    }
}

#[cfg(test)]
mod tests;
