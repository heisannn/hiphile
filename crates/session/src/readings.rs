use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use executor::{Executor, Saver, Task};
use futures::future::{Either, select};
use fuzzy::Reader;
use fuzzy::bundle::{self, BUNDLE};
use tokio::sync::{broadcast, watch};
use url::Url;

const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);
const NOTICES: usize = 16;

#[derive(Clone, Debug, PartialEq)]
pub enum ReadingsState {
    Off,
    Checking,
    /// As a fraction from 0 to 1.
    Downloading(f32),
    Ready(PathBuf),
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReadingsStatus {
    state: ReadingsState,
    /// Why the choice to turn them on or off could not be read or carried out.
    choice_error: Option<String>,
}

impl ReadingsStatus {
    pub fn state(&self) -> &ReadingsState {
        &self.state
    }

    pub fn is_on(&self) -> bool {
        self.state != ReadingsState::Off
    }

    pub fn choice_error(&self) -> Option<&str> {
        self.choice_error.as_deref()
    }

    /// Where the dictionaries are, once they are there.
    pub fn dictionaries(&self) -> Option<&Path> {
        match &self.state {
            ReadingsState::Ready(dir) => Some(dir),
            _ => None,
        }
    }

    /// What to add when a search finds nothing but might have found kanji with
    /// dictionaries that are not there, or romaji would look like it does not work.
    pub fn missing_note(&self) -> Option<&'static str> {
        match self.state {
            ReadingsState::Checking | ReadingsState::Downloading(_) => {
                Some("Finding kanji by romaji needs a dictionary that has yet to download.")
            }
            ReadingsState::Failed(_) => Some(
                "Finding kanji by romaji needs a dictionary that failed to download. \
                 Try again in Settings.",
            ),
            ReadingsState::Off | ReadingsState::Ready(_) => None,
        }
    }
}

/// Sent each time the dictionaries cannot be had, even when the state was already a failure.
#[derive(Clone, Debug)]
pub struct ReadingsFailed {
    error: String,
}

impl ReadingsFailed {
    pub fn error(&self) -> &str {
        &self.error
    }
}

/// The dictionaries that read kanji: fetched in the background the first time
/// the app runs, unless the user turned them off.
///
/// Lists work without them, finding text as written and by its kana. They read
/// their text again when the dictionaries come or go. Dropping the last handle
/// stops a download under way.
#[derive(Clone)]
pub struct Readings(Arc<Inner>);

struct WeakReadings(Weak<Inner>);

struct Inner {
    executor: Executor,
    net: net::Client,
    /// `None` where the system has no cache directory.
    root: Option<PathBuf>,
    /// `None` without a home directory: a choice then lasts until quitting.
    saver: Option<Saver<bool>>,
    state: Mutex<State>,
    published: watch::Sender<ReadingsStatus>,
    notices: broadcast::Sender<ReadingsFailed>,
}

struct State {
    status: ReadingsStatus,
    /// Counts the fetches and removals, so that one replaced while it was finishing on another
    /// thread cannot change the state after its successor has.
    work_id: u64,
    _work: Task<()>,
}

impl Readings {
    pub fn new(
        executor: Executor,
        net: net::Client,
        store: Result<config::Store, config::Error>,
    ) -> Self {
        let (store, is_on, choice_error) = read_saved_choice(store);
        Self::with(
            executor,
            net,
            store,
            bundle::root().ok(),
            is_on,
            choice_error,
        )
    }

    fn with(
        executor: Executor,
        net: net::Client,
        store: Option<config::Store>,
        root: Option<PathBuf>,
        is_on: bool,
        choice_error: Option<String>,
    ) -> Self {
        let status = ReadingsStatus {
            state: ReadingsState::Off,
            choice_error,
        };
        let inner = Arc::new_cyclic(|this: &Weak<Inner>| Inner {
            saver: store.map(|store| {
                let this = WeakReadings(this.clone());
                Saver::new(executor.clone(), move |is_on| {
                    let Err(error) = store.save_romaji_search(is_on) else {
                        return;
                    };
                    if let Some(this) = this.upgrade() {
                        this.0.update(|state| {
                            let error = format!("Couldn't save the search setting: {error}.");
                            state.status.choice_error = Some(error);
                        });
                    }
                })
            }),
            executor,
            net,
            root,
            published: watch::Sender::new(status.clone()),
            notices: broadcast::Sender::new(NOTICES),
            state: Mutex::new(State {
                status,
                work_id: 0,
                _work: Task::ready(()),
            }),
        });
        let this = Self(inner);
        this.0.update(|state| {
            if is_on {
                this.start_fetch(state);
            } else {
                // Off may have been chosen in the file, or as a download finished
                this.start_remove(state);
            }
        });
        this
    }

    pub fn status(&self) -> ReadingsStatus {
        lock(&self.0.state).status.clone()
    }

    /// Sees each new status. A borrow of it must not be held while calling the readings, which
    /// publish under their own lock.
    pub fn subscribe(&self) -> watch::Receiver<ReadingsStatus> {
        self.0.published.subscribe()
    }

    /// Hears each failure from now on.
    pub fn notices(&self) -> broadcast::Receiver<ReadingsFailed> {
        self.0.notices.subscribe()
    }

    /// Turns the search by romaji on, fetching the dictionaries, or off,
    /// removing them.
    pub fn set_on(&self, is_on: bool) {
        self.0.update(|state| {
            if state.status.is_on() == is_on {
                return;
            }
            state.status.choice_error = None;
            // Under the lock, so that the choices reach the saver in the order they were made
            if let Some(saver) = &self.0.saver {
                saver.save(is_on);
            }
            if is_on {
                self.start_fetch(state);
            } else {
                self.0.set_state(state, ReadingsState::Off);
                self.start_remove(state);
            }
        });
    }

    pub fn retry(&self) {
        self.0.update(|state| {
            if matches!(state.status.state, ReadingsState::Failed(_)) {
                self.start_fetch(state);
            }
        });
    }

    fn start_fetch(&self, state: &mut State) {
        let work_id = state.next_work();
        let Some(root) = self.0.root.clone() else {
            let error = format!(
                "Couldn't download the dictionaries: {}.",
                bundle::Error::NoCacheDir
            );
            self.0.set_state(state, ReadingsState::Failed(error));
            return;
        };
        self.0.set_state(state, ReadingsState::Checking);
        let this = self.downgrade();
        let executor = self.0.executor.clone();
        let net = self.0.net.clone();
        state._work = self.0.executor.spawn(async move {
            let saved = executor
                .spawn_blocking({
                    let root = root.clone();
                    move || usable_bundle(&root)
                })
                .await;
            let result = match saved {
                Some(dir) => Ok(dir),
                None => download(&this, work_id, &executor, net, root).await,
            };
            let state = match result {
                Ok(dir) => ReadingsState::Ready(dir),
                Err(error) => ReadingsState::Failed(error),
            };
            this.set_state(work_id, state);
        });
    }

    /// Removes the dictionaries, stopping a download under way.
    fn start_remove(&self, state: &mut State) {
        let work_id = state.next_work();
        let Some(root) = self.0.root.clone() else {
            return;
        };
        let this = self.downgrade();
        let executor = self.0.executor.clone();
        state._work = self.0.executor.spawn(async move {
            let removed = executor.spawn_blocking(move || bundle::remove(&root)).await;
            let Err(error) = removed else {
                return;
            };
            this.change(work_id, |_, state| {
                let error = format!("Couldn't remove the dictionaries: {error}.");
                state.status.choice_error = Some(error);
            });
        });
    }

    fn downgrade(&self) -> WeakReadings {
        WeakReadings(Arc::downgrade(&self.0))
    }
}

impl State {
    fn next_work(&mut self) -> u64 {
        self.work_id += 1;
        self.work_id
    }
}

impl WeakReadings {
    fn upgrade(&self) -> Option<Readings> {
        self.0.upgrade().map(Readings)
    }

    /// Changes the state unless the work `work_id` has been replaced.
    fn change(&self, work_id: u64, change: impl FnOnce(&Inner, &mut State)) {
        if let Some(this) = self.upgrade() {
            this.0.update(|state| {
                if state.work_id == work_id {
                    change(&this.0, state);
                }
            });
        }
    }

    fn set_state(&self, work_id: u64, new: ReadingsState) {
        self.change(work_id, |inner, state| inner.set_state(state, new));
    }
}

impl Inner {
    /// Publishes before letting go of the lock, so that an older status cannot follow a newer
    /// one.
    fn update<R>(&self, change: impl FnOnce(&mut State) -> R) -> R {
        let mut state = lock(&self.state);
        let result = change(&mut state);
        self.published.send_if_modified(|published| {
            let is_new = *published != state.status;
            if is_new {
                published.clone_from(&state.status);
            }
            is_new
        });
        result
    }

    fn set_state(&self, state: &mut State, new: ReadingsState) {
        if let ReadingsState::Failed(error) = &new {
            // Fails only when nobody listens
            self.notices
                .send(ReadingsFailed {
                    error: error.clone(),
                })
                .ok();
        }
        state.status.state = new;
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The saved choice, which is on when it cannot be read, and why it could not be
/// read or will not be saved.
fn read_saved_choice(
    store: Result<config::Store, config::Error>,
) -> (Option<config::Store>, bool, Option<String>) {
    match store {
        Ok(store) => match store.romaji_search() {
            Ok(is_on) => (Some(store), is_on, None),
            Err(error) => {
                let error = format!("Couldn't read the search setting: {error}.");
                (Some(store), true, Some(error))
            }
        },
        Err(error) => {
            let error = format!("The search setting won't be saved: {error}.");
            (None, true, Some(error))
        }
    }
}

/// The bundle under `root`, unless it cannot be opened, as when the system cleared
/// part of its cache. Then it goes, to be downloaded again.
fn usable_bundle(root: &Path) -> Option<PathBuf> {
    let dir = bundle::installed(root, &BUNDLE)?;
    if Reader::open(&dir).is_ok() {
        return Some(dir);
    }
    bundle::remove(root).ok();
    None
}

/// Stops a download when the work waiting on it goes away, which its blocking
/// reads would not notice.
struct CancelOnDrop(net::DownloadHandle);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

async fn download(
    this: &WeakReadings,
    work_id: u64,
    executor: &Executor,
    net: net::Client,
    root: PathBuf,
) -> Result<PathBuf, String> {
    let describe = |error: net::Error| format!("Couldn't download the dictionaries: {error}.");
    let url = Url::parse(BUNDLE.url)
        .map_err(|error| format!("Couldn't download the dictionaries: {error}."))?;
    let download = net.download(url).await.map_err(describe)?;
    let total = download.content_length().unwrap_or(BUNDLE.download_bytes);
    let _cancel = CancelOnDrop(download.handle());
    let read = Arc::new(AtomicU64::new(0));
    let mut install = executor.spawn_blocking({
        let read = read.clone();
        move || {
            bundle::install(download, &root, &BUNDLE, move |bytes| {
                read.store(bytes, Ordering::Relaxed);
            })
        }
    });
    loop {
        let timer = executor.timer(PROGRESS_INTERVAL);
        if let Either::Left((result, _)) = select(&mut install, timer).await {
            return result.map_err(|error| format!("Couldn't install the dictionaries: {error}."));
        }
        let fraction = fraction_of(read.load(Ordering::Relaxed), total);
        this.set_state(work_id, ReadingsState::Downloading(fraction));
    }
}

fn fraction_of(read: u64, total: u64) -> f32 {
    if total == 0 {
        return 0.;
    }
    (read as f32 / total as f32).clamp(0., 1.)
}

#[cfg(test)]
mod tests;
