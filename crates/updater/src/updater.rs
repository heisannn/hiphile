use std::sync::{Arc, Weak};
use std::time::Duration;

use executor::{Executor, Publish, Published, Saver, Task};
use tokio::sync::watch;

use crate::install::{Installation, Outcome, Releases};

const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateState {
    Idle,
    Checking,
    UpToDate,
    /// Found while updates are not automatic, and waits for [`Updater::install`].
    Available(String),
    Installing(String),
    /// On disk, for the next launch. There is nothing more to look for until then.
    Ready(String),
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    state: UpdateState,
    is_automatic: bool,
    setting_error: Option<String>,
}

impl Status {
    pub fn state(&self) -> &UpdateState {
        &self.state
    }

    pub fn is_automatic(&self) -> bool {
        self.is_automatic
    }

    pub fn errors(&self) -> impl Iterator<Item = &str> {
        let failure = match &self.state {
            UpdateState::Failed(error) => Some(error),
            _ => None,
        };
        failure
            .into_iter()
            .chain(&self.setting_error)
            .map(String::as_str)
    }
}

/// The running app keeps its code until it restarts, so nothing playing stops. Dropping the
/// last handle stops looking.
#[derive(Clone)]
pub struct Updater(Arc<Inner>);

struct WeakUpdater(Weak<Inner>);

struct Inner {
    executor: Executor,
    releases: Arc<dyn Releases>,
    /// `None` without a home directory: the choice then lasts until quitting.
    saver: Option<Saver<bool>>,
    state: Published<State>,
}

struct State {
    status: Status,
    _check: Task<()>,
    _install: Task<()>,
    _schedule: Task<()>,
}

impl Updater {
    /// `None` in a build that does not update itself, or on a platform with no releases.
    pub fn of_this_app(
        executor: Executor,
        store: Result<config::Store, config::Error>,
    ) -> Option<Self> {
        option_env!("HIPHILE_SELF_UPDATE")?;
        let installation = Installation::of_this_app()?;
        let (store, is_automatic, setting_error) = read_saved_choice(store);
        Some(Self::with(
            executor,
            Arc::new(installation),
            store,
            is_automatic,
            setting_error,
        ))
    }

    fn with(
        executor: Executor,
        releases: Arc<dyn Releases>,
        store: Option<config::Store>,
        is_automatic: bool,
        setting_error: Option<String>,
    ) -> Self {
        let status = Status {
            state: UpdateState::Idle,
            is_automatic,
            setting_error,
        };
        let inner = Arc::new_cyclic(|this: &Weak<Inner>| Inner {
            saver: store.map(|store| {
                let this = WeakUpdater(this.clone());
                Saver::new(executor.clone(), move |is_on| {
                    let saved = store.save_auto_update(is_on);
                    if let Some(this) = this.upgrade() {
                        this.0.update(|state| {
                            state.status.setting_error = saved
                                .err()
                                .map(|error| format!("Couldn't save the choice: {error}."));
                        });
                    }
                })
            }),
            executor,
            releases,
            state: Published::new(State {
                status,
                _check: Task::ready(()),
                _install: Task::ready(()),
                _schedule: Task::ready(()),
            }),
        });
        let this = Self(inner);
        this.schedule();
        this
    }

    pub fn status(&self) -> Status {
        self.0.state.snapshot()
    }

    /// Sees each new status. A borrow of it must not be held while calling the updater, which
    /// publishes under its own lock.
    pub fn subscribe(&self) -> watch::Receiver<Status> {
        self.0.state.subscribe()
    }

    /// Installs what it finds only if updates are automatic by the time it has looked.
    pub fn check(&self) {
        self.0.update(|state| self.start_check(state));
    }

    pub fn install(&self) {
        self.0.update(|state| self.start_install(state));
    }

    /// Turning this on looks for a new version right away. Turning it off does not stop an
    /// install already under way, which may be replacing the app.
    pub fn set_automatic(&self, is_on: bool) {
        self.0.update(|state| {
            if state.status.is_automatic == is_on {
                return;
            }
            state.status.is_automatic = is_on;
            // Under the lock, so that the choices reach the saver in the order they were made
            if let Some(saver) = &self.0.saver {
                saver.save(is_on);
            }
            if is_on {
                if matches!(state.status.state, UpdateState::Available(_)) {
                    self.start_install(state);
                } else {
                    self.start_check(state);
                }
            }
        });
    }

    fn start_check(&self, state: &mut State) {
        if matches!(
            state.status.state,
            UpdateState::Checking | UpdateState::Installing(_) | UpdateState::Ready(_)
        ) {
            return;
        }
        state.status.state = UpdateState::Checking;
        let this = self.downgrade();
        let releases = self.0.releases.clone();
        let executor = self.0.executor.clone();
        state._check = self.0.executor.spawn(async move {
            let found = executor
                .spawn_blocking(move || releases.newer_version())
                .await;
            let Some(this) = this.upgrade() else {
                return;
            };
            this.0.update(|state| {
                state.status.state = match found {
                    Ok(None) => UpdateState::UpToDate,
                    Ok(Some(version)) => UpdateState::Available(version),
                    Err(error) => UpdateState::Failed(error),
                };
                if state.status.is_automatic {
                    this.start_install(state);
                }
            });
        });
    }

    fn start_install(&self, state: &mut State) {
        let UpdateState::Available(version) = &state.status.state else {
            return;
        };
        state.status.state = UpdateState::Installing(version.clone());
        let this = self.downgrade();
        let releases = self.0.releases.clone();
        let executor = self.0.executor.clone();
        state._install = self.0.executor.spawn(async move {
            let outcome = executor.spawn_blocking(move || releases.install()).await;
            let Some(this) = this.upgrade() else {
                return;
            };
            this.0.update(|state| {
                state.status.state = match outcome {
                    Ok(Outcome::UpToDate) => UpdateState::UpToDate,
                    Ok(Outcome::Installed(version)) => UpdateState::Ready(version),
                    Err(error) => UpdateState::Failed(error),
                };
            });
        });
    }

    fn schedule(&self) {
        let this = self.downgrade();
        let executor = self.0.executor.clone();
        let schedule = self.0.executor.spawn(async move {
            loop {
                match this.upgrade() {
                    Some(this) => this.0.update(|state| {
                        if state.status.is_automatic {
                            this.start_check(state);
                        }
                    }),
                    None => return,
                }
                executor.timer(CHECK_INTERVAL).await;
            }
        });
        self.0.update(|state| state._schedule = schedule);
    }

    fn downgrade(&self) -> WeakUpdater {
        WeakUpdater(Arc::downgrade(&self.0))
    }
}

impl WeakUpdater {
    fn upgrade(&self) -> Option<Updater> {
        self.0.upgrade().map(Updater)
    }
}

impl Inner {
    fn update<R>(&self, change: impl FnOnce(&mut State) -> R) -> R {
        self.state.update(change)
    }
}

impl Publish for State {
    type Snapshot = Status;

    fn snapshot(&self) -> &Status {
        &self.status
    }
}

/// Not automatic when the choice cannot be read, as it may have been turned off.
fn read_saved_choice(
    store: Result<config::Store, config::Error>,
) -> (Option<config::Store>, bool, Option<String>) {
    let store = match store {
        Ok(store) => store,
        Err(error) => {
            let error = format!("The update choice won't be saved: {error}.");
            return (None, true, Some(error));
        }
    };
    match store.auto_update() {
        Ok(is_on) => (Some(store), is_on, None),
        Err(error) => {
            let error = format!("Couldn't read the update choice: {error}.");
            (Some(store), false, Some(error))
        }
    }
}

#[cfg(test)]
mod tests;
