use std::sync::{Arc, Weak};

use executor::{Executor, Saver};
use tokio::sync::watch;

use executor::{Publish, Published};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueSnapshot {
    is_skip_instrumentals_on: bool,
    error: Option<String>,
}

impl QueueSnapshot {
    pub fn is_skip_instrumentals_on(&self) -> bool {
        self.is_skip_instrumentals_on
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

impl Publish for QueueSnapshot {
    type Snapshot = Self;

    fn snapshot(&self) -> &Self {
        self
    }
}

#[derive(Clone)]
pub struct QueueSettings(Arc<Inner>);

struct Inner {
    /// `None` without a home directory: the choice then lasts until quitting.
    saver: Option<Saver<bool>>,
    state: Published<QueueSnapshot>,
}

impl QueueSettings {
    pub fn new(executor: Executor, store: Result<config::Store, config::Error>) -> Self {
        let (store, is_on, error) = read_saved(store);
        Self::with(executor, store, is_on, error)
    }

    fn with(
        executor: Executor,
        store: Option<config::Store>,
        is_on: bool,
        error: Option<String>,
    ) -> Self {
        Self(Arc::new_cyclic(|this: &Weak<Inner>| Inner {
            saver: store.map(|store| {
                let this = this.clone();
                Saver::new(executor, move |is_on| {
                    let saved = store.save_skip_instrumentals(is_on);
                    if let Some(this) = this.upgrade() {
                        this.state.update(|state| {
                            state.error = saved
                                .err()
                                .map(|error| format!("Couldn't save this setting: {error}."));
                        });
                    }
                })
            }),
            state: Published::new(QueueSnapshot {
                is_skip_instrumentals_on: is_on,
                error,
            }),
        }))
    }

    pub fn snapshot(&self) -> QueueSnapshot {
        self.0.state.snapshot()
    }

    /// Sees each new snapshot. A borrow of it must not be held while calling these settings,
    /// which publish under their own lock.
    pub fn subscribe(&self) -> watch::Receiver<QueueSnapshot> {
        self.0.state.subscribe()
    }

    pub fn set_skip_instrumentals(&self, is_on: bool) {
        self.0.state.update(|state| {
            if state.is_skip_instrumentals_on == is_on {
                return;
            }
            state.is_skip_instrumentals_on = is_on;
            // Under the lock, so that the choices reach the saver in the order they were made
            if let Some(saver) = &self.0.saver {
                saver.save(is_on);
            }
        });
    }
}

/// The saved choice, and why it could not be read or will not be saved.
fn read_saved(
    store: Result<config::Store, config::Error>,
) -> (Option<config::Store>, bool, Option<String>) {
    let store = match store {
        Ok(store) => store,
        Err(error) => {
            let error = format!("This setting won't be saved: {error}.");
            return (None, false, Some(error));
        }
    };
    match store.skip_instrumentals() {
        Ok(is_on) => (Some(store), is_on, None),
        Err(error) => {
            let error = format!("Couldn't read this setting: {error}.");
            (Some(store), false, Some(error))
        }
    }
}

#[cfg(test)]
mod tests {
    use executor::testing::TestExecutor;

    use super::*;

    #[test]
    fn a_new_choice_is_told_and_saved_and_the_same_one_is_neither() {
        let cx = &TestExecutor::new();
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let executor = cx.executor();
        let settings = QueueSettings::with(executor, Some(store.clone()), false, None);
        let told = settings.subscribe();

        settings.set_skip_instrumentals(false);
        cx.run_until_parked();
        assert!(!told.has_changed().unwrap());
        assert!(!store.config_path().exists());

        settings.set_skip_instrumentals(true);
        cx.run_until_parked();
        assert!(told.has_changed().unwrap());
        assert!(store.skip_instrumentals().unwrap());
        let settings = settings.snapshot();
        assert!(settings.is_skip_instrumentals_on());
        assert!(settings.error().is_none());
    }
}
