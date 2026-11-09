use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use executor::{Executor, Task};
use playback::PlayerNotice;
use session::Session;

use crate::favorites::FavoritesState;
use crate::follow;
use crate::home::DailyRecommendations;
use crate::library::{LibraryActivity, LibraryNotice, LibrarySync};
use crate::playlists::PlaylistsState;
use crate::search::SearchIndex;
use crate::settings::{AudioSnapshot, Settings};

/// Everything that exists only while someone is signed in, and what passes changes between it
/// and the settings. Made on signing in; dropping the last handle stops what it does, playing
/// included.
#[derive(Clone)]
pub struct Workspace(Arc<Inner>);

struct Inner {
    profile: config::Profile,
    session: Session,
    library: LibrarySync,
    activity: LibraryActivity,
    search: SearchIndex,
    recommendations: DailyRecommendations,
    favorites: FavoritesState,
    playlists: PlaylistsState,
    player: playback::Player,
    lyrics: lyrics::CurrentLyrics,
    executor: Executor,
    settings: Settings,
    /// For the desktop's followers.
    #[cfg(feature = "desktop")]
    covers: Option<catalog::CoverStore>,
    routes: Mutex<Vec<Task<()>>>,
}

/// What the workspace is made with besides its session.
pub(crate) struct Parts {
    pub(crate) executor: Executor,
    pub(crate) settings: Settings,
    /// Holds the volume.
    pub(crate) store: Result<config::Store, config::Error>,
    pub(crate) covers: Option<catalog::CoverStore>,
    pub(crate) song_cache: Option<playback::SongCache>,
}

impl Workspace {
    pub(crate) fn new(session: Session, profile: config::Profile, parts: Parts) -> Self {
        let Parts {
            executor,
            settings,
            store,
            covers,
            song_cache,
        } = parts;
        let library = LibrarySync::new(executor.clone(), session.clone());
        let activity = LibraryActivity::new(executor.clone(), session.clone(), &library);
        let audio = settings.audio().snapshot();
        let player = playback::Player::new(
            executor.clone(),
            session.clone(),
            store,
            audio.output().clone(),
            song_cache,
        );
        player.set_transitions(audio.transitions());
        #[cfg(not(feature = "desktop"))]
        drop(covers);
        let finder = lyrics::Finder::user(session.client().net().clone());
        let this = Self(Arc::new(Inner {
            profile,
            library,
            activity,
            search: SearchIndex::new(executor.clone(), session.clone()),
            recommendations: DailyRecommendations::new(executor.clone(), session.clone()),
            favorites: FavoritesState::new(executor.clone(), session.client().clone()),
            playlists: PlaylistsState::new(executor.clone(), session.clone()),
            player,
            lyrics: lyrics::CurrentLyrics::new(executor.clone(), finder),
            session,
            executor,
            settings,
            #[cfg(feature = "desktop")]
            covers,
            routes: Mutex::default(),
        }));
        let routes = vec![
            this.follow_library(),
            this.follow_plays(),
            this.follow_audio_settings(audio),
            this.follow_queue_settings(),
            this.follow_lyrics_settings(),
        ];
        this.keep(routes);
        this
    }

    pub fn profile(&self) -> &config::Profile {
        &self.0.profile
    }

    pub fn session(&self) -> &Session {
        &self.0.session
    }

    pub fn library(&self) -> &LibrarySync {
        &self.0.library
    }

    pub fn activity(&self) -> &LibraryActivity {
        &self.0.activity
    }

    pub fn search(&self) -> &SearchIndex {
        &self.0.search
    }

    pub fn recommendations(&self) -> &DailyRecommendations {
        &self.0.recommendations
    }

    pub fn favorites(&self) -> &FavoritesState {
        &self.0.favorites
    }

    pub fn playlists(&self) -> &PlaylistsState {
        &self.0.playlists
    }

    pub fn player(&self) -> &playback::Player {
        &self.0.player
    }

    /// The lyrics of what the player plays, once a view looks them up.
    pub fn lyrics(&self) -> &lyrics::CurrentLyrics {
        &self.0.lyrics
    }

    #[cfg(feature = "desktop")]
    pub(crate) fn executor(&self) -> &Executor {
        &self.0.executor
    }

    #[cfg(feature = "desktop")]
    pub(crate) fn settings(&self) -> &Settings {
        &self.0.settings
    }

    #[cfg(feature = "desktop")]
    pub(crate) fn covers(&self) -> Option<&catalog::CoverStore> {
        self.0.covers.as_ref()
    }

    /// Keeps `routes` going for as long as the workspace lives.
    pub(crate) fn keep(&self, routes: impl IntoIterator<Item = Task<()>>) {
        lock(&self.0.routes).extend(routes);
    }

    /// What a sync brought reaches the search and the recommendations, which may now have
    /// something to pick from.
    fn follow_library(&self) -> Task<()> {
        let search = self.0.search.clone();
        let recommendations = self.0.recommendations.clone();
        follow::subscribe(
            &self.0.executor,
            self.0.library.notices(),
            move |notice| match notice {
                LibraryNotice::AlbumsSynced => {
                    recommendations.library_synced();
                    search.library_changed();
                }
                LibraryNotice::ArtistsSynced | LibraryNotice::SongsSynced => {
                    search.library_changed();
                }
                LibraryNotice::SyncFailed(_)
                | LibraryNotice::SyncFinished
                | LibraryNotice::StageSynced(..) => {}
            },
        )
    }

    /// A play the server has heard of may change what is worth recommending.
    fn follow_plays(&self) -> Task<()> {
        let recommendations = self.0.recommendations.clone();
        follow::subscribe(
            &self.0.executor,
            self.0.player.notices(),
            move |notice| match notice {
                PlayerNotice::Played => recommendations.renew(),
            },
        )
    }

    /// The settings also change for what only their window shows, such as the devices listed,
    /// which the player hears nothing of.
    fn follow_audio_settings(&self, audio: AudioSnapshot) -> Task<()> {
        let (mut output, mut transitions) = (audio.output().clone(), audio.transitions());
        let player = self.0.player.clone();
        follow::observe(
            &self.0.executor,
            self.0.settings.audio().subscribe(),
            move |audio: AudioSnapshot| {
                if *audio.output() != output {
                    output = audio.output().clone();
                    player.set_output(output.clone());
                }
                if audio.transitions() != transitions {
                    transitions = audio.transitions();
                    player.set_transitions(transitions);
                }
            },
        )
    }

    fn follow_queue_settings(&self) -> Task<()> {
        let changes = self.0.settings.queue().subscribe();
        let mut is_on = changes.borrow().is_skip_instrumentals_on();
        self.0.player.set_skip_instrumentals(is_on);
        let player = self.0.player.clone();
        follow::observe(&self.0.executor, changes, move |queue| {
            if queue.is_skip_instrumentals_on() != is_on {
                is_on = queue.is_skip_instrumentals_on();
                player.set_skip_instrumentals(is_on);
            }
        })
    }

    fn follow_lyrics_settings(&self) -> Task<()> {
        let changes = self.0.settings.lyrics().subscribe();
        let mut highlight = changes.borrow().highlight();
        self.0.lyrics.set_highlight(highlight);
        let lyrics = self.0.lyrics.clone();
        follow::observe(&self.0.executor, changes, move |settings| {
            if settings.highlight() != highlight {
                highlight = settings.highlight();
                lyrics.set_highlight(highlight);
            }
        })
    }

    pub(crate) fn set_song_cache(&self, song_cache: Option<playback::SongCache>) {
        self.0.player.set_song_cache(song_cache);
    }

    /// Restores the output device as the player found it.
    pub(crate) fn shutdown(&self) {
        self.0.player.shutdown();
    }
}

/// The same workspace, not another one for the same account.
impl PartialEq for Workspace {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Workspace {}

impl fmt::Debug for Workspace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Workspace")
            .field("profile", &self.0.profile)
            .finish_non_exhaustive()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests;
