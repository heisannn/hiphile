use std::sync::Arc;
use std::time::Duration;

use gpui_kit::{AppContext as _, Context, SharedString, Task};

use crate::install::{Outcome, Releases};

const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateState {
    Idle,
    Checking,
    UpToDate,
    /// Found while updates are not automatic, and waits for [`Updater::install`].
    Available(SharedString),
    Installing(SharedString),
    /// On disk, for the next launch. There is nothing more to look for until then.
    Ready(SharedString),
    Failed(SharedString),
}

/// The running app keeps its code until it restarts, so nothing playing stops.
pub struct Updater {
    releases: Arc<dyn Releases>,
    state: UpdateState,
    is_automatic: bool,
    /// `None` without a home directory: the choice then lasts until quitting.
    store: Option<config::Store>,
    setting_error: Option<SharedString>,
    /// Only a change by the user is written: the value may have come from the environment,
    /// or from a file that could not be read.
    unsaved: bool,
    saving: bool,
    _save: Task<()>,
    _check: Task<()>,
    _install: Task<()>,
    _schedule: Task<()>,
}

impl Updater {
    pub(crate) fn new(releases: Arc<dyn Releases>, cx: &mut Context<Self>) -> Self {
        let (store, is_automatic, setting_error) = read_saved_choice();
        Self::with(releases, store, is_automatic, setting_error, cx)
    }

    fn with(
        releases: Arc<dyn Releases>,
        store: Option<config::Store>,
        is_automatic: bool,
        setting_error: Option<SharedString>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            releases,
            state: UpdateState::Idle,
            is_automatic,
            store,
            setting_error,
            unsaved: false,
            saving: false,
            _save: Task::ready(()),
            _check: Task::ready(()),
            _install: Task::ready(()),
            _schedule: cx.spawn(async move |this, cx| {
                while this
                    .update(cx, |this, cx| {
                        if this.is_automatic {
                            this.check(cx);
                        }
                    })
                    .is_ok()
                {
                    cx.background_executor().timer(CHECK_INTERVAL).await;
                }
            }),
        }
    }

    pub fn state(&self) -> &UpdateState {
        &self.state
    }

    pub fn is_automatic(&self) -> bool {
        self.is_automatic
    }

    pub fn errors(&self) -> impl Iterator<Item = &SharedString> {
        let failure = match &self.state {
            UpdateState::Failed(error) => Some(error),
            _ => None,
        };
        failure.into_iter().chain(&self.setting_error)
    }

    /// Installs what it finds only if updates are automatic by the time it has looked.
    pub fn check(&mut self, cx: &mut Context<Self>) {
        if matches!(
            self.state,
            UpdateState::Checking | UpdateState::Installing(_) | UpdateState::Ready(_)
        ) {
            return;
        }
        self.state = UpdateState::Checking;
        cx.notify();
        let releases = self.releases.clone();
        self._check = cx.spawn(async move |this, cx| {
            let found = cx
                .background_spawn(async move { releases.newer_version() })
                .await;
            this.update(cx, |this, cx| {
                this.state = match found {
                    Ok(None) => UpdateState::UpToDate,
                    Ok(Some(version)) => UpdateState::Available(version.into()),
                    Err(error) => UpdateState::Failed(error.into()),
                };
                if this.is_automatic {
                    this.install(cx);
                }
                cx.notify();
            })
            .ok();
        });
    }

    pub fn install(&mut self, cx: &mut Context<Self>) {
        let UpdateState::Available(version) = &self.state else {
            return;
        };
        self.state = UpdateState::Installing(version.clone());
        cx.notify();
        let releases = self.releases.clone();
        self._install = cx.spawn(async move |this, cx| {
            let outcome = cx.background_spawn(async move { releases.install() }).await;
            this.update(cx, |this, cx| {
                this.state = match outcome {
                    Ok(Outcome::UpToDate) => UpdateState::UpToDate,
                    Ok(Outcome::Installed(version)) => UpdateState::Ready(version.into()),
                    Err(error) => UpdateState::Failed(error.into()),
                };
                cx.notify();
            })
            .ok();
        });
    }

    /// Turning this on looks for a new version right away. Turning it off does not stop an
    /// install already under way, which may be replacing the app.
    pub fn set_automatic(&mut self, is_on: bool, cx: &mut Context<Self>) {
        if self.is_automatic == is_on {
            return;
        }
        self.is_automatic = is_on;
        self.unsaved = true;
        self.save(cx);
        if is_on {
            if matches!(self.state, UpdateState::Available(_)) {
                self.install(cx);
            } else {
                self.check(cx);
            }
        }
        cx.notify();
    }

    /// One write at a time, so the last choice is the one that stays.
    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        if self.saving {
            return;
        }
        self.saving = true;
        self._save = cx.spawn(async move |this, cx| {
            while let Ok(Some(is_on)) = this.update(cx, |this, _| {
                this.saving = std::mem::take(&mut this.unsaved);
                this.saving.then_some(this.is_automatic)
            }) {
                let store = store.clone();
                let saved = cx
                    .background_spawn(async move { store.save_auto_update(is_on) })
                    .await;
                this.update(cx, |this, cx| {
                    this.setting_error = saved
                        .err()
                        .map(|error| format!("Couldn't save the choice: {error}.").into());
                    cx.notify();
                })
                .ok();
            }
        });
    }
}

/// Not automatic when the choice cannot be read, as it may have been turned off.
fn read_saved_choice() -> (Option<config::Store>, bool, Option<SharedString>) {
    let store = match config::Store::user() {
        Ok(store) => store,
        Err(error) => {
            let error = format!("The update choice won't be saved: {error}.");
            return (None, true, Some(error.into()));
        }
    };
    match store.auto_update() {
        Ok(is_on) => (Some(store), is_on, None),
        Err(error) => {
            let error = format!("Couldn't read the update choice: {error}.");
            (Some(store), false, Some(error.into()))
        }
    }
}

#[cfg(test)]
mod tests;
