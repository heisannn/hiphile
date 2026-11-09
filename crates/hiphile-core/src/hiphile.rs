use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};

use executor::{Executor, Publish, Published, Task};
use session::{Accounts, Invalid, Readings, Session, SignedIn};
use subsonic::Credentials;
use tokio::sync::watch;

use crate::follow;
use crate::settings::{CacheLimit, CacheLimitSnapshot, Settings, SongCacheUsage};
use crate::workspace::{self, Parts, Workspace};

/// Where this device keeps what it downloads: covers, songs, the dictionaries that read kanji
/// and Musixmatch's token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Caches {
    /// A folder of the app's in the system's cache directory.
    User,
    /// In a folder the host gives, such as a mobile app's caches directory.
    At(PathBuf),
    /// Nowhere: covers and songs come from the server each time, the dictionaries cannot be had,
    /// and Musixmatch's token lasts until quitting.
    None,
}

impl Caches {
    /// `None` also where the system has no cache directory.
    fn folder(self) -> Option<PathBuf> {
        match self {
            Self::User => dirs::cache_dir().map(|cache| cache.join("hiphile")),
            Self::At(folder) => Some(folder),
            Self::None => None,
        }
    }
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
    net: net::Client,
    /// Holds the settings and the profiles; the player keeps the volume in it.
    store: Result<config::Store, Arc<config::Error>>,
    /// `None` when there is nowhere to keep anything.
    cache: Option<PathBuf>,
    settings: Settings,
    /// `None` when there is nowhere to keep them.
    dictionaries: Option<PathBuf>,
    /// Made only when needed, so that a host that searches nothing, such as a command line,
    /// neither fetches nor removes the dictionaries.
    readings: OnceLock<Readings>,
    /// `None` when there is nowhere to keep them.
    covers: Option<catalog::CoverStore>,
    /// What the player's song cache writes to, for as long as the core lives. `None` when there
    /// is nowhere to keep them.
    songs: Option<catalog::SongStore>,
    accounts: Accounts,
    state: Published<State>,
    follows: Mutex<Vec<Task<()>>>,
}

struct State {
    shown: HiphileSnapshot,
    /// `None` when there is nowhere to keep songs or their limit could not be read.
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
        let cache = caches.folder();
        let covers = cache
            .as_deref()
            .map(|cache| session::open_covers(share(&store), cache));
        let dictionaries = cache.as_deref().map(fuzzy::bundle::root_in);
        let song_limit = settings.song_cache().snapshot();
        let songs = cache
            .as_deref()
            .map(|cache| catalog::SongStore::in_cache(cache, song_capacity(song_limit.capacity())));
        // A limit that cannot be read keeps no songs rather than the default amount
        let song_cache = songs
            .clone()
            .filter(|_| !song_limit.is_keeping_none())
            .map(playback::SongCache::new);
        let accounts = Accounts::new(executor.clone(), net.clone(), share(&store), keychain);
        let this = Self(Arc::new(Inner {
            state: Published::new(State {
                shown: HiphileSnapshot::default(),
                song_cache,
            }),
            executor,
            net,
            store,
            cache,
            settings,
            dictionaries,
            readings: OnceLock::new(),
            covers,
            songs,
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

    /// The first call, or signing in, makes them, which starts fetching the dictionaries, or
    /// removing them when they are off.
    pub fn readings(&self) -> &Readings {
        self.0.readings.get_or_init(|| {
            Readings::at(
                self.0.executor.clone(),
                self.0.net.clone(),
                share(&self.0.store),
                self.0.dictionaries.clone(),
            )
        })
    }

    /// Where covers are kept, if anywhere.
    pub fn covers(&self) -> Option<&catalog::CoverStore> {
        self.0.covers.as_ref()
    }

    /// How much the song cache keeps, counted now, and removing it. Each call counts anew.
    pub fn song_cache_usage(&self) -> SongCacheUsage {
        SongCacheUsage::new(self.0.executor.clone(), self.0.songs.clone())
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
    /// is remembered, no workspace opens and the session has no dictionaries, as for a command
    /// line.
    pub fn connect(
        &self,
        address: &str,
        credentials: Credentials,
    ) -> Result<Task<Result<Session, String>>, Invalid> {
        self.0.accounts.connect(address, credentials)
    }

    /// A player on `session` for a host that opens no workspace, such as a command line. It plays
    /// on the output and with the transitions that the settings hold now, at the saved volume,
    /// and keeps songs as the workspace's player does, but follows no later change of the
    /// settings. The host calls [`playback::Player::shutdown`] once it is done with it.
    pub fn player(&self, session: Session) -> playback::Player {
        let song_cache = self.0.state.read(|state| state.song_cache.clone());
        workspace::new_player(
            &self.0.executor,
            session,
            share(&self.0.store),
            &self.0.settings.audio().snapshot(),
            song_cache,
        )
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
        let session = session.with_readings(self.readings().clone());
        self.0.state.update(|state| {
            let parts = Parts {
                executor: self.0.executor.clone(),
                settings: self.0.settings.clone(),
                store: share(&self.0.store),
                cache: self.0.cache.clone(),
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

    /// The songs keep to the new limit from the next one kept. A player that kept none, as the
    /// limit could not be read, starts keeping them from its next song.
    fn follow_song_limit(&self) -> Task<()> {
        let this = self.downgrade();
        follow_limit(
            &self.0.executor,
            self.0.settings.song_cache(),
            move |capacity| {
                let Some(this) = this.upgrade() else {
                    return;
                };
                let Some(songs) = &this.0.songs else {
                    return;
                };
                songs.set_capacity(song_capacity(capacity));
                this.0.state.update(|state| {
                    if state.song_cache.is_some() {
                        return;
                    }
                    let song_cache = playback::SongCache::new(songs.clone());
                    state.song_cache = Some(song_cache.clone());
                    if let Some(workspace) = &state.shown.workspace {
                        workspace.set_song_cache(Some(song_cache));
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

/// The default amount for `None`.
fn song_capacity(capacity: Option<u64>) -> u64 {
    capacity.unwrap_or(catalog::SONG_CAPACITY)
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
