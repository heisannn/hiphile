use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use executor::{Executor, Publish, Published, Task};
use session::{Accounts, Invalid, Readings, Session, SignedIn};
use subsonic::Credentials;
use tokio::sync::watch;

use crate::follow;
use crate::settings::{CacheLimit, CacheLimitSnapshot, Settings};
use crate::workspace::{Parts, Workspace};

/// Where this device keeps what it downloads: covers, songs and the dictionaries that read kanji.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Caches {
    /// The system's cache directory.
    User,
    /// Nowhere: covers and songs come from the server each time, and the dictionaries cannot be
    /// had.
    None,
}

/// Whether someone is signed in, and as whom.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HiphileSnapshot {
    workspace: Option<Workspace>,
}

impl HiphileSnapshot {
    /// What exists while someone is signed in.
    pub fn workspace(&self) -> Option<&Workspace> {
        self.workspace.as_ref()
    }

    /// Who is signed in.
    pub fn profile(&self) -> Option<&config::Profile> {
        self.workspace.as_ref().map(Workspace::profile)
    }
}

/// The app apart from its windows: the settings, the dictionaries, the covers and songs kept,
/// signing in, and the workspace of whoever is signed in. A host makes one when it starts, and
/// keeps it for as long as it runs.
#[derive(Clone)]
pub struct Hiphile(Arc<Inner>);

struct WeakHiphile(Weak<Inner>);

struct Inner {
    executor: Executor,
    /// Holds the settings and the profiles; the player keeps the volume in it.
    store: Result<config::Store, Arc<config::Error>>,
    caches: Caches,
    settings: Settings,
    readings: Readings,
    /// `None` when there is nowhere to keep them.
    covers: Option<catalog::CoverStore>,
    accounts: Accounts,
    state: Published<State>,
    follows: Mutex<Vec<Task<()>>>,
}

struct State {
    shown: HiphileSnapshot,
    /// `None` when there is nowhere to keep them or their limit cannot be read.
    song_cache: Option<playback::SongCache>,
}

impl Publish for State {
    type Snapshot = HiphileSnapshot;

    fn snapshot(&self) -> &HiphileSnapshot {
        &self.shown
    }
}

impl Hiphile {
    /// `net` is the one client of the process (it must not be dropped last inside a tokio
    /// runtime), `store` holds the settings and the profiles, and `keychain` the passwords. Reads
    /// the settings and opens the caches before it returns.
    pub fn new(
        executor: Executor,
        net: net::Client,
        store: Result<config::Store, config::Error>,
        keychain: Option<config::Keychain>,
        caches: Caches,
    ) -> Self {
        let store = store.map_err(Arc::new);
        let settings = Settings::new(executor.clone(), share(&store));
        let (covers, dictionaries) = match caches {
            Caches::User => (
                session::open_covers(share(&store)),
                fuzzy::bundle::root().ok(),
            ),
            Caches::None => (None, None),
        };
        let song_limit = settings.song_cache().snapshot();
        // A limit that cannot be read keeps no songs rather than the default amount
        let song_cache = if song_limit.is_keeping_none() {
            None
        } else {
            open_song_cache(caches, song_limit.capacity())
        };
        let readings = Readings::at(executor.clone(), net.clone(), share(&store), dictionaries);
        let accounts = Accounts::new(
            executor.clone(),
            net,
            share(&store),
            keychain,
            Some(readings.clone()),
        );
        let this = Self(Arc::new(Inner {
            state: Published::new(State {
                shown: HiphileSnapshot::default(),
                song_cache,
            }),
            executor,
            store,
            caches,
            settings,
            readings,
            covers,
            accounts,
            follows: Mutex::default(),
        }));
        let follows = [this.follow_cover_limit(), this.follow_song_limit()];
        lock(&this.0.follows).extend(follows);
        this
    }

    pub fn settings(&self) -> &Settings {
        &self.0.settings
    }

    pub fn readings(&self) -> &Readings {
        &self.0.readings
    }

    /// Where covers are kept, if anywhere.
    pub fn covers(&self) -> Option<&catalog::CoverStore> {
        self.0.covers.as_ref()
    }

    pub fn snapshot(&self) -> HiphileSnapshot {
        self.0.state.snapshot()
    }

    /// Sees each new snapshot. A borrow of it must not be held while calling the core, which
    /// publishes under its own lock.
    pub fn subscribe(&self) -> watch::Receiver<HiphileSnapshot> {
        self.0.state.subscribe()
    }

    /// The workspace of whoever is signed in.
    pub fn workspace(&self) -> Option<Workspace> {
        self.0.state.read(|state| state.shown.workspace.clone())
    }

    /// The profile last signed in with.
    pub fn last_profile(&self) -> Option<config::Profile> {
        self.0.accounts.last_profile()
    }

    /// The password written in the settings file for `profile`, or else the one in the keychain,
    /// which may ask the user first. `None` when the keychain fails.
    pub fn saved_password(&self, profile: config::Profile) -> Task<Option<String>> {
        self.0.accounts.saved_password(profile)
    }

    /// Checks what was typed before anything else. Signed in, the workspace opens and the profile
    /// and the password are remembered for next time; the error says what went wrong for the
    /// user.
    pub fn sign_in(
        &self,
        address: &str,
        username: &str,
        password: &str,
    ) -> Result<Task<Result<(), String>>, Invalid> {
        let attempt = self.0.accounts.sign_in(address, username, password)?;
        let this = self.downgrade();
        Ok(self.0.executor.spawn(async move {
            let SignedIn {
                session, profile, ..
            } = attempt.await?;
            if let Some(this) = this.upgrade() {
                this.open_workspace(session, profile);
            }
            Ok(())
        }))
    }

    /// Opens a session on the server at `address` with `credentials` without signing in: nothing
    /// is remembered and no workspace opens, as for a command line.
    pub fn connect(
        &self,
        address: &str,
        credentials: Credentials,
    ) -> Result<Task<Result<Session, String>>, Invalid> {
        self.0.accounts.connect(address, credentials)
    }

    /// Closes the workspace, which stops playing, and forgets the saved password, so that signing
    /// in again asks for it, unless the settings file has it.
    pub fn sign_out(&self) {
        let closed = self.0.state.update(|state| state.shown.workspace.take());
        if let Some(closed) = closed {
            self.0.accounts.forget_password(closed.profile().clone());
        }
    }

    /// Writes what is still waiting to be saved and restores the output device as the player
    /// found it. Call when the host quits, which may not drop the workspace.
    pub fn shutdown(&self) {
        self.0.settings.save_before_quitting();
        if let Some(workspace) = self.workspace() {
            workspace.shutdown();
        }
    }

    pub(crate) fn open_workspace(&self, session: Session, profile: config::Profile) {
        self.0.state.update(|state| {
            let parts = Parts {
                executor: self.0.executor.clone(),
                settings: self.0.settings.clone(),
                store: share(&self.0.store),
                covers: self.0.covers.clone(),
                song_cache: state.song_cache.clone(),
            };
            state.shown.workspace = Some(Workspace::new(session, profile, parts));
        });
    }

    fn follow_cover_limit(&self) -> Task<()> {
        let Some(covers) = self.0.covers.clone() else {
            return Task::ready(());
        };
        follow_limit(
            &self.0.executor,
            self.0.settings.cover_cache(),
            move |capacity| {
                session::set_cover_capacity(&covers, capacity);
            },
        )
    }

    /// The cache opens again for the new limit, which the player keeps to from its next song.
    fn follow_song_limit(&self) -> Task<()> {
        let this = self.downgrade();
        let caches = self.0.caches;
        follow_limit(
            &self.0.executor,
            self.0.settings.song_cache(),
            move |capacity| {
                let Some(this) = this.upgrade() else {
                    return;
                };
                let song_cache = open_song_cache(caches, capacity);
                this.0.state.update(|state| {
                    state.song_cache.clone_from(&song_cache);
                    if let Some(workspace) = &state.shown.workspace {
                        workspace.set_song_cache(song_cache);
                    }
                });
            },
        )
    }

    fn downgrade(&self) -> WeakHiphile {
        WeakHiphile(Arc::downgrade(&self.0))
    }
}

impl WeakHiphile {
    fn upgrade(&self) -> Option<Hiphile> {
        self.0.upgrade().map(Hiphile)
    }
}

/// Each handle takes its own store, and says itself why it won't be saved without one.
#[expect(
    clippy::result_large_err,
    reason = "what every constructor takes, as `config::Store::user` gives it"
)]
pub(crate) fn share(
    store: &Result<config::Store, Arc<config::Error>>,
) -> Result<config::Store, config::Error> {
    store.clone().map_err(config::Error::Shared)
}

/// Keeps at most about `capacity` bytes of songs, or the default amount for `None`. Without the
/// cache every song comes from the server, so failing to open it raises no error.
fn open_song_cache(caches: Caches, capacity: Option<u64>) -> Option<playback::SongCache> {
    match caches {
        Caches::User => playback::SongCache::user(capacity).ok(),
        Caches::None => None,
    }
}

/// Only a change of the limit in effect reaches `set_capacity`, which reopens the cache: the
/// setting also changes as the limit is typed and when a save fails.
fn follow_limit(
    executor: &Executor,
    limit: &CacheLimit,
    mut set_capacity: impl FnMut(Option<u64>) + Send + 'static,
) -> Task<()> {
    let in_effect = |limit: &CacheLimitSnapshot| (limit.capacity(), limit.is_keeping_none());
    let changes = limit.subscribe();
    let mut passed_on = in_effect(&changes.borrow());
    follow::observe(executor, changes, move |limit| {
        if in_effect(&limit) != passed_on {
            passed_on = in_effect(&limit);
            set_capacity(limit.capacity());
        }
    })
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests;
