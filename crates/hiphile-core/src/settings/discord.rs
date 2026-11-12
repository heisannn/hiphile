use std::sync::{Arc, Weak};
use std::time::Duration;

use executor::{Executor, Saver, Task};
use tokio::sync::watch;

use executor::{Publish, Published};

/// The field reports every keystroke, and each key on the way would be tried on ImgBB.
const SETTLE: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscordSnapshot {
    /// The key in effect.
    key: Option<String>,
    is_presence_on: bool,
    /// Why the key could not be read or saved.
    key_error: Option<String>,
    presence_error: Option<String>,
    upload_error: Option<String>,
}

impl DiscordSnapshot {
    pub fn imgbb_key(&self) -> Option<&str> {
        self.key.as_deref()
    }

    pub fn is_presence_on(&self) -> bool {
        self.is_presence_on
    }

    pub fn errors(&self) -> impl Iterator<Item = &str> {
        self.key_error
            .iter()
            .chain(&self.presence_error)
            .chain(&self.upload_error)
            .map(String::as_str)
    }
}

/// The Discord status, and the ImgBB API key that lets covers go on it.
#[derive(Clone)]
pub struct DiscordSettings(Arc<Inner>);

struct Inner {
    executor: Executor,
    /// `None` without a home directory: the choices then last until quitting.
    store: Option<config::Store>,
    key_saver: Option<Saver<Option<String>>>,
    presence_saver: Option<Saver<bool>>,
    state: Published<State>,
}

struct State {
    discord: DiscordSnapshot,
    /// What the field last held, which the key becomes once it settles. `None` when it is
    /// empty.
    typed: Option<String>,
    _settle: Task<()>,
}

impl Publish for State {
    type Snapshot = DiscordSnapshot;

    fn snapshot(&self) -> &DiscordSnapshot {
        &self.discord
    }
}

impl DiscordSettings {
    pub fn new(executor: Executor, store: Result<config::Store, config::Error>) -> Self {
        let (store, key, key_error, is_presence_on, presence_error) = read_saved(store);
        Self::with(
            executor,
            store,
            DiscordSnapshot {
                key,
                is_presence_on,
                key_error,
                presence_error,
                upload_error: None,
            },
        )
    }

    fn with(executor: Executor, store: Option<config::Store>, discord: DiscordSnapshot) -> Self {
        Self(Arc::new_cyclic(|this: &Weak<Inner>| Inner {
            key_saver: store.clone().map(|store| {
                let this = this.clone();
                Saver::new(executor.clone(), move |key: Option<String>| {
                    let saved = store.save_imgbb_api_key(key.as_deref());
                    if let Some(this) = this.upgrade() {
                        this.state.update(|state| {
                            state.discord.key_error = saved
                                .err()
                                .map(|error| format!("Couldn't save the API key: {error}."));
                        });
                    }
                })
            }),
            presence_saver: store.clone().map(|store| {
                let this = this.clone();
                Saver::new(executor.clone(), move |is_on| {
                    let saved = store.save_discord_presence(is_on);
                    if let Some(this) = this.upgrade() {
                        this.state.update(|state| {
                            state.discord.presence_error = saved
                                .err()
                                .map(|error| format!("Couldn't save the Discord status: {error}."));
                        });
                    }
                })
            }),
            executor,
            store,
            state: Published::new(State {
                typed: discord.key.clone(),
                discord,
                _settle: Task::ready(()),
            }),
        }))
    }

    pub fn snapshot(&self) -> DiscordSnapshot {
        self.0.state.snapshot()
    }

    /// Sees each new snapshot. A borrow of it must not be held while calling these settings,
    /// which publish under their own lock.
    pub fn subscribe(&self) -> watch::Receiver<DiscordSnapshot> {
        self.0.state.subscribe()
    }

    /// `error` says why a cover could not be uploaded, or `None` once one was. An upload that was
    /// under way when the status was turned off no longer matters.
    pub fn set_upload_error(&self, error: Option<&str>) {
        self.0.state.update(|state| {
            state.discord.upload_error = error
                .filter(|_| state.discord.is_presence_on)
                .map(|error| format!("Couldn't upload the cover: {error}."));
        });
    }

    pub fn set_presence_on(&self, is_on: bool) {
        self.0.state.update(|state| {
            let discord = &mut state.discord;
            if discord.is_presence_on == is_on {
                return;
            }
            discord.is_presence_on = is_on;
            if !is_on {
                discord.upload_error = None;
            }
            // Under the lock, so that the choices reach the saver in the order they were made
            if let Some(saver) = &self.0.presence_saver {
                saver.save(is_on);
            }
        });
    }

    /// The key becomes what is left of `text` once no other has come for a moment.
    pub fn set_typed(&self, text: &str) {
        let this = Arc::downgrade(&self.0);
        let executor = self.0.executor.clone();
        self.0.state.update(|state| {
            state.typed = chosen_key(text);
            state._settle = self.0.executor.spawn(async move {
                executor.timer(SETTLE).await;
                if let Some(this) = this.upgrade() {
                    DiscordSettings(this).apply();
                }
            });
        });
    }

    fn apply(&self) {
        self.0.state.update(|state| {
            if state.typed == state.discord.key {
                return;
            }
            let discord = &mut state.discord;
            discord.key = state.typed.clone();
            if self.0.store.is_some() {
                discord.key_error = None;
            }
            discord.upload_error = None;
            // Under the lock, so that the keys reach the saver in the order they were applied
            if let Some(saver) = &self.0.key_saver {
                saver.save(discord.key.clone());
            }
        });
    }

    /// Writes what is still settling or waiting for its turn, which nothing would write once the
    /// app has quit. Blocks on the disk.
    pub fn save_before_quitting(&self) {
        let (Some(store), Some(key_saver), Some(presence_saver)) =
            (&self.0.store, &self.0.key_saver, &self.0.presence_saver)
        else {
            return;
        };
        let (typed, key) = self
            .0
            .state
            .read(|state| (state.typed.clone(), state.discord.key.clone()));
        let waiting_key = key_saver.take_waiting();
        if typed != key {
            store.save_imgbb_api_key(typed.as_deref()).ok();
        } else if let Some(key) = waiting_key {
            store.save_imgbb_api_key(key.as_deref()).ok();
        }
        if let Some(is_on) = presence_saver.take_waiting() {
            store.save_discord_presence(is_on).ok();
        }
    }
}

/// Pasting often brings spaces along.
fn chosen_key(text: &str) -> Option<String> {
    let key = text.trim();
    (!key.is_empty()).then(|| key.to_owned())
}

/// The saved key and whether the status is on, and why each could not be read or will not be
/// saved. The status is not on when it cannot be read, as it may have been turned off.
fn read_saved(
    store: Result<config::Store, config::Error>,
) -> (
    Option<config::Store>,
    Option<String>,
    Option<String>,
    bool,
    Option<String>,
) {
    let store = match store {
        Ok(store) => store,
        Err(error) => {
            let key_error = format!("The API key won't be saved: {error}.");
            let presence_error = format!("The Discord status won't be saved: {error}.");
            return (None, None, Some(key_error), true, Some(presence_error));
        }
    };
    let (key, key_error) = match store.imgbb_api_key() {
        Ok(key) => (key, None),
        Err(error) => (None, Some(format!("Couldn't read the API key: {error}."))),
    };
    let (is_presence_on, presence_error) = match store.discord_presence() {
        Ok(is_on) => (is_on, None),
        Err(error) => (
            false,
            Some(format!("Couldn't read the Discord status: {error}.")),
        ),
    };
    (Some(store), key, key_error, is_presence_on, presence_error)
}

#[cfg(test)]
mod tests {
    use executor::testing::TestExecutor;

    use super::*;

    fn presence(is_on: bool) -> DiscordSnapshot {
        DiscordSnapshot {
            key: None,
            is_presence_on: is_on,
            key_error: None,
            presence_error: None,
            upload_error: None,
        }
    }

    #[test]
    fn turning_the_status_off_is_told_and_saved_and_doing_so_twice_is_not() {
        let cx = &TestExecutor::new();
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let executor = cx.executor();
        let settings = DiscordSettings::with(executor, Some(store.clone()), presence(true));
        let mut told = settings.subscribe();

        settings.set_presence_on(true);
        cx.run_until_parked();
        assert!(!told.has_changed().unwrap());
        assert!(!store.config_path().exists());

        settings.set_presence_on(false);
        cx.run_until_parked();
        assert!(!told.borrow_and_update().is_presence_on());
        assert!(!store.discord_presence().unwrap());
    }

    #[test]
    fn with_the_status_off_a_late_upload_error_is_not_shown() {
        let cx = &TestExecutor::new();
        let executor = cx.executor();
        let settings = DiscordSettings::with(executor, None, presence(false));

        settings.set_upload_error(Some("gone"));

        assert_eq!(settings.snapshot().errors().count(), 0);
    }

    #[test]
    fn a_key_is_what_is_left_of_the_text() {
        assert_eq!(chosen_key("  abc123\n").as_deref(), Some("abc123"));
        assert_eq!(chosen_key(""), None);
        assert_eq!(chosen_key("   "), None);
    }
}
