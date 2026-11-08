use std::sync::{Arc, Weak};

use config::Appearance;
use executor::{Executor, Saver};
use tokio::sync::watch;

use executor::{Publish, Published};

/// `None` follows the system.
pub const APPEARANCES: [(Option<Appearance>, &str); 3] = [
    (None, "System"),
    (Some(Appearance::Light), "Light"),
    (Some(Appearance::Dark), "Dark"),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppearanceSnapshot {
    appearance: Option<Appearance>,
    error: Option<String>,
}

impl AppearanceSnapshot {
    /// `None` follows the system.
    pub fn appearance(&self) -> Option<Appearance> {
        self.appearance
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

impl Publish for AppearanceSnapshot {
    type Snapshot = Self;

    fn snapshot(&self) -> &Self {
        self
    }
}

/// The chosen appearance. Applying it to the UI is the host's.
#[derive(Clone)]
pub struct AppearanceSettings(Arc<Inner>);

struct Inner {
    /// `None` without a home directory: the choice then lasts until quitting.
    saver: Option<Saver<Option<Appearance>>>,
    state: Published<AppearanceSnapshot>,
}

impl AppearanceSettings {
    pub fn new(executor: Executor, store: Result<config::Store, config::Error>) -> Self {
        let (store, appearance, error) = read_saved_appearance(store);
        Self(Arc::new_cyclic(|this: &Weak<Inner>| Inner {
            saver: store.map(|store| {
                let this = this.clone();
                Saver::new(executor, move |appearance| {
                    let saved = store.save_appearance(appearance);
                    if let Some(this) = this.upgrade() {
                        this.state.update(|state| {
                            state.error = saved
                                .err()
                                .map(|error| format!("Couldn't save the appearance: {error}."));
                        });
                    }
                })
            }),
            state: Published::new(AppearanceSnapshot { appearance, error }),
        }))
    }

    pub fn snapshot(&self) -> AppearanceSnapshot {
        self.0.state.snapshot()
    }

    /// Sees each new snapshot. A borrow of it must not be held while calling these settings,
    /// which publish under their own lock.
    pub fn subscribe(&self) -> watch::Receiver<AppearanceSnapshot> {
        self.0.state.subscribe()
    }

    /// `None` follows the system.
    pub fn set_appearance(&self, appearance: Option<Appearance>) {
        self.0.state.update(|state| {
            if state.appearance == appearance {
                return;
            }
            state.appearance = appearance;
            // Under the lock, so that the choices reach the saver in the order they were made
            if let Some(saver) = &self.0.saver {
                saver.save(appearance);
            }
        });
    }
}

/// The saved appearance, and why it could not be read or will not be saved.
fn read_saved_appearance(
    store: Result<config::Store, config::Error>,
) -> (Option<config::Store>, Option<Appearance>, Option<String>) {
    let store = match store {
        Ok(store) => store,
        Err(error) => {
            let error = format!("The appearance won't be saved: {error}.");
            return (None, None, Some(error));
        }
    };
    match store.appearance() {
        Ok(appearance) => (Some(store), appearance, None),
        Err(error) => {
            let error = format!("Couldn't read the appearance: {error}.");
            (Some(store), None, Some(error))
        }
    }
}
