use std::mem;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};

use executor::{Executor, Publish, Published, Task};
use session::{Accounts, Invalid, Readings, Session, SignedIn};
use subsonic::Credentials;
use tokio::sync::watch;

use crate::alerts::{self, Alerts};
use crate::covers;
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

/// Whether someone is signed in, as whom, and with whom else they could be.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HiphileSnapshot {
    workspace: Option<Workspace>,
    is_signing_in: bool,
    profiles: Arc<[config::Profile]>,
}

impl HiphileSnapshot {
    /// What exists while someone is signed in.
    pub fn workspace(&self) -> Option<&Workspace> {
        self.workspace.as_ref()
    }

    /// Whether an attempt to sign in is under way, which takes no other until it ends.
    pub fn is_signing_in(&self) -> bool {
        self.is_signing_in
    }

    /// Who is signed in.
    pub fn profile(&self) -> Option<&config::Profile> {
        self.workspace.as_ref().map(Workspace::profile)
    }

    /// Those signed in with and not signed out of since, in the order first signed in with, the
    /// one signed in among them.
    pub fn profiles(&self) -> &[config::Profile] {
        &self.profiles
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
    /// Of what the app does apart from a workspace: the dictionaries.
    alerts: alerts::Collector,
    state: Published<State>,
    follows: Mutex<Vec<Task<()>>>,
}

struct State {
    shown: HiphileSnapshot,
    /// `None` when there is nowhere to keep songs or their limit could not be read.
    song_cache: Option<playback::SongCache>,
    /// Counts the attempts to sign in, the switches and signing out, so that one that ends late
    /// cannot end its successor, and a switch whose password comes late cannot start after them.
    attempt_id: u64,
    /// Of the attempt under way.
    calling_off: Option<futures::future::AbortHandle>,
    /// Whether whoever signed in last may still be signed in again without asking: once, and
    /// not after signing out, as a password in the settings file would sign them straight back in.
    may_resume: bool,
}

/// Why signing in did not start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignInRefused {
    /// What was typed, found before asking any server.
    Invalid(Invalid),
    /// Another attempt is under way, which decides.
    SigningIn,
}

/// Why switching to a profile did not sign in with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SwitchFailed {
    /// None is saved for it, so only a typed one signs in with it.
    NoPassword,
    /// What went wrong signing in, for the user.
    SignIn(String),
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
                shown: HiphileSnapshot {
                    profiles: accounts.profiles().into(),
                    ..HiphileSnapshot::default()
                },
                song_cache,
                attempt_id: 0,
                calling_off: None,
                may_resume: true,
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
            alerts: alerts::Collector::new(),
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
            let readings = Readings::at(
                self.0.executor.clone(),
                self.0.net.clone(),
                share(&self.0.store),
                self.0.dictionaries.clone(),
            );
            let failures = alerts::follow_readings(&self.0.executor, &readings, &self.0.alerts);
            lock(&self.0.follows).push(failures);
            readings
        })
    }

    /// What the user should hear of apart from a workspace, which has its own
    /// ([`Workspace::alerts`]): those told since the core was made, and those to come. Asking
    /// does not make the readings.
    pub fn alerts(&self) -> Alerts {
        self.0.alerts.follow()
    }

    /// Where covers are kept, if anywhere.
    pub fn covers(&self) -> Option<&catalog::CoverStore> {
        self.0.covers.as_ref()
    }

    /// The cover at `url`, a session's [`Session::cover_art_url`], from the covers kept while they
    /// have it and else from the server, or nothing when neither has it.
    pub fn cover_image(&self, url: &str) -> Task<Option<Arc<[u8]>>> {
        covers::cover_image(
            &self.0.executor,
            self.0.net.clone(),
            self.0.covers.clone(),
            Arc::from(url),
        )
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

    /// Checks what was typed before anything else, and takes no attempt while another is under
    /// way. Signed in, the workspace opens in place of any other, and the profile and the
    /// password are remembered for next time; the error says what went wrong for the user.
    /// Dropping the task calls the attempt off, and signing out does too, which ends it with `Ok`.
    pub fn sign_in(
        &self,
        address: &str,
        username: &str,
        password: &str,
    ) -> Result<Task<Result<(), String>>, SignInRefused> {
        self.0
            .state
            .update(|state| self.start_signing_in(state, address, username, password))
    }

    fn start_signing_in(
        &self,
        state: &mut State,
        address: &str,
        username: &str,
        password: &str,
    ) -> Result<Task<Result<(), String>>, SignInRefused> {
        if state.shown.is_signing_in {
            return Err(SignInRefused::SigningIn);
        }
        let attempt = self
            .0
            .accounts
            .sign_in(address, username, password)
            .map_err(SignInRefused::Invalid)?;
        let (attempt, calling_off) = futures::future::abortable(attempt);
        state.shown.is_signing_in = true;
        state.attempt_id += 1;
        state.calling_off = Some(calling_off);
        let ending = Attempt {
            hiphile: self.downgrade(),
            id: state.attempt_id,
        };
        Ok(self.0.executor.spawn(async move {
            let Ok(signed_in) = attempt.await else {
                return Ok(());
            };
            let SignedIn {
                session, profile, ..
            } = signed_in?;
            if let Some(this) = ending.hiphile.upgrade() {
                this.signed_in(ending.id, session, profile);
            }
            Ok(())
        }))
    }

    /// Signs in as whoever did last, as [`Self::switch_to`] does, once for as long as the core
    /// lives. `None` after the first call, after signing out, and when nobody has signed in
    /// before. Ends with `Ok` also when no password was saved. Dropping the task calls it off, as
    /// when the user starts typing instead.
    pub fn resume(&self) -> Option<Task<Result<(), String>>> {
        if !self
            .0
            .state
            .update(|state| mem::take(&mut state.may_resume))
        {
            return None;
        }
        let switching = self.switch_to(self.0.accounts.last_profile()?);
        Some(self.0.executor.spawn(async move {
            match switching.await {
                Ok(()) | Err(SwitchFailed::NoPassword) => Ok(()),
                Err(SwitchFailed::SignIn(error)) => Err(error),
            }
        }))
    }

    /// Signs in with `profile` and the password saved for it, as [`Self::sign_in`] does, in place
    /// of whoever is signed in, who stays until it succeeds. Ends with `Ok` at once when `profile`
    /// is signed in already or another attempt is under way, which decides, and without signing in
    /// when signing in, switching or signing out is asked for before the password is found.
    /// Dropping the task calls it off.
    pub fn switch_to(&self, profile: config::Profile) -> Task<Result<(), SwitchFailed>> {
        let request = self.0.state.update(|state| {
            let decided = state.shown.profile() == Some(&profile) || state.shown.is_signing_in;
            (!decided).then(|| {
                state.attempt_id += 1;
                state.attempt_id
            })
        });
        let Some(request) = request else {
            return Task::ready(Ok(()));
        };
        let lookup = self.0.accounts.saved_password(profile.clone());
        let this = self.downgrade();
        self.0.executor.spawn(async move {
            let password = lookup.await;
            let Some(this) = this.upgrade() else {
                return Ok(());
            };
            let attempt = this.0.state.update(|state| {
                if state.attempt_id != request {
                    return Ok(None);
                }
                let password = password.ok_or(SwitchFailed::NoPassword)?;
                match this.start_signing_in(state, profile.server(), profile.username(), &password)
                {
                    Ok(attempt) => Ok(Some(attempt)),
                    Err(SignInRefused::SigningIn) => Ok(None),
                    Err(SignInRefused::Invalid(invalid)) => {
                        Err(SwitchFailed::SignIn(invalid.message().to_owned()))
                    }
                }
            })?;
            match attempt {
                Some(attempt) => attempt.await.map_err(SwitchFailed::SignIn),
                None => Ok(()),
            }
        })
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
    /// on the output and with the transitions and DSP that the settings hold now, at the saved
    /// volume, and keeps songs as the workspace's player does, but follows no later change of the
    /// settings. The host calls [`playback::Player::shutdown`] once it is done with it.
    pub fn player(&self, session: Session) -> playback::Player {
        let song_cache = self.0.state.read(|state| state.song_cache.clone());
        let player = workspace::new_player(
            &self.0.executor,
            session,
            share(&self.0.store),
            &self.0.settings.audio().snapshot(),
            song_cache,
        );
        player.set_dsp(self.0.settings.dsp().snapshot().settings());
        player
    }

    /// Calls off signing in and closes the workspace, which stops playing, takes its profile off
    /// the profiles and forgets its saved password, so that signing in again asks for it, unless
    /// the settings file has it. Nothing is resumed after it.
    pub fn sign_out(&self) {
        self.0.state.update(|state| {
            state.may_resume = false;
            state.attempt_id += 1;
            state.shown.is_signing_in = false;
            if let Some(calling_off) = state.calling_off.take() {
                calling_off.abort();
            }
            if let Some(closed) = state.shown.workspace.take() {
                self.0.accounts.forget(closed.profile().clone());
                state.shown.profiles = self.0.accounts.profiles().into();
            }
        });
    }

    /// Takes `profile` off the profiles and forgets its saved password, as signing out does,
    /// unless it is signed in.
    pub fn forget(&self, profile: config::Profile) {
        self.0.state.update(|state| {
            if state.shown.profile() != Some(&profile) {
                self.0.accounts.forget(profile);
                state.shown.profiles = self.0.accounts.profiles().into();
            }
        });
    }

    /// Writes what is still waiting to be saved, restores the output device as the player found
    /// it and clears the system's media controls. Call when the host quits, which may not drop
    /// the workspace.
    pub fn shutdown(&self) {
        self.0.settings.save_before_quitting();
        if let Some(workspace) = self.workspace() {
            workspace.shutdown();
        }
    }

    #[cfg(test)]
    pub(crate) fn open_workspace(&self, session: Session, profile: config::Profile) {
        let session = session.with_readings(self.readings().clone());
        self.0
            .state
            .update(|state| self.open_workspace_in(state, session, profile));
    }

    /// Opens the workspace and ends the attempt `attempt_id` in one change, so that no host sees
    /// it signed in while still signing in.
    fn signed_in(&self, attempt_id: u64, session: Session, profile: config::Profile) {
        let session = session.with_readings(self.readings().clone());
        self.0.state.update(|state| {
            state.shown.profiles = self.0.accounts.profiles().into();
            // Not when signing out called it off as it ended
            if state.attempt_id == attempt_id {
                state.shown.is_signing_in = false;
                self.open_workspace_in(state, session, profile);
            }
        });
    }

    fn open_workspace_in(&self, state: &mut State, session: Session, profile: config::Profile) {
        let parts = Parts {
            executor: self.0.executor.clone(),
            settings: self.0.settings.clone(),
            store: share(&self.0.store),
            cache: self.0.cache.clone(),
            covers: self.0.covers.clone(),
            song_cache: state.song_cache.clone(),
        };
        state.shown.workspace = Some(Workspace::new(session, profile, parts));
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

/// Ends an attempt to sign in however it ends, also when its task is dropped unfinished.
struct Attempt {
    hiphile: WeakHiphile,
    id: u64,
}

impl Drop for Attempt {
    fn drop(&mut self) {
        if let Some(this) = self.hiphile.upgrade() {
            this.0.state.update(|state| {
                if state.attempt_id == self.id {
                    state.shown.is_signing_in = false;
                }
            });
        }
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
