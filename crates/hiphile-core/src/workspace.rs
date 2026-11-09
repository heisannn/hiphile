use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use executor::{Executor, Task};
use playback::PlayerNotice;
use session::Session;
use tokio::sync::watch;

use crate::alerts::{self, Alerts};
use crate::artists::{ArtistDetail, ArtistList};
use crate::favorites::FavoritesState;
use crate::follow;
use crate::home::{DailyRecommendations, HomeShelves};
use crate::library::{AlbumList, LibraryActivity, LibraryNotice, LibrarySync};
use crate::navigation::Destination;
use crate::playlists::PlaylistsState;
use crate::search::SearchIndex;
use crate::settings::{AudioSnapshot, Settings};
use crate::songs::SongList;

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
    albums: AlbumList,
    search: SearchIndex,
    recommendations: DailyRecommendations,
    shelves: HomeShelves,
    artists: ArtistList,
    artist_detail: ArtistDetail,
    songs: SongList,
    favorites: FavoritesState,
    playlists: PlaylistsState,
    player: playback::Player,
    lyrics: lyrics::CurrentLyrics,
    executor: Executor,
    settings: Settings,
    alerts: alerts::Collector,
    /// For the desktop's followers.
    #[cfg(feature = "desktop")]
    covers: Option<catalog::CoverStore>,
    /// Cleared on shutdown: quitting may not drop them, and the system keeps what they showed.
    #[cfg(feature = "desktop")]
    media_controls: Mutex<Vec<crate::desktop::WeakControls>>,
    routes: Mutex<Vec<Task<()>>>,
}

/// What the workspace is made with besides its session.
pub(crate) struct Parts {
    pub(crate) executor: Executor,
    pub(crate) settings: Settings,
    /// Holds the volume.
    pub(crate) store: Result<config::Store, config::Error>,
    /// Where Musixmatch keeps its token.
    pub(crate) cache: Option<PathBuf>,
    pub(crate) covers: Option<catalog::CoverStore>,
    pub(crate) song_cache: Option<playback::SongCache>,
}

impl Workspace {
    pub(crate) fn new(session: Session, profile: config::Profile, parts: Parts) -> Self {
        let Parts {
            executor,
            settings,
            store,
            cache,
            covers,
            song_cache,
        } = parts;
        let library = LibrarySync::new(executor.clone(), session.clone());
        let activity = LibraryActivity::new(executor.clone(), session.clone(), &library);
        let albums = AlbumList::new(executor.clone(), session.clone(), &library);
        // Followed from before the player is made, so that a change while the rest is made
        // reaches it
        let audio_changes = settings.audio().subscribe();
        let audio = audio_changes.borrow().clone();
        let player = new_player(&executor, session.clone(), store, &audio, song_cache);
        #[cfg(not(feature = "desktop"))]
        drop(covers);
        let finder = lyrics::Finder::with_cache(session.client().net().clone(), cache.as_deref());
        let recommendations = DailyRecommendations::new(executor.clone(), session.clone());
        let this = Self(Arc::new(Inner {
            profile,
            library,
            activity,
            albums,
            search: SearchIndex::new(executor.clone(), session.clone()),
            shelves: HomeShelves::new(executor.clone(), session.clone(), recommendations.clone()),
            recommendations,
            artists: ArtistList::new(executor.clone(), session.clone()),
            artist_detail: ArtistDetail::new(executor.clone(), session.clone()),
            songs: SongList::new(executor.clone(), session.clone()),
            favorites: FavoritesState::new(executor.clone(), session.client().clone()),
            playlists: PlaylistsState::new(executor.clone(), session.clone()),
            player,
            lyrics: lyrics::CurrentLyrics::new(executor.clone(), finder),
            session,
            executor,
            settings,
            alerts: alerts::Collector::new(),
            #[cfg(feature = "desktop")]
            covers,
            #[cfg(feature = "desktop")]
            media_controls: Mutex::default(),
            routes: Mutex::default(),
        }));
        let routes = vec![
            this.follow_library(),
            this.follow_plays(),
            this.follow_audio_settings(audio_changes, &audio),
            this.follow_queue_settings(),
            this.follow_lyrics_settings(),
            this.follow_songs_for_lyrics(),
        ];
        this.keep(routes);
        this.keep(this.collect_alerts());
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

    /// The albums the library keeps, as their screen arranges them.
    pub fn albums(&self) -> &AlbumList {
        &self.0.albums
    }

    pub fn search(&self) -> &SearchIndex {
        &self.0.search
    }

    pub fn recommendations(&self) -> &DailyRecommendations {
        &self.0.recommendations
    }

    /// The home screen's shelves besides the recommendations.
    pub fn shelves(&self) -> &HomeShelves {
        &self.0.shelves
    }

    pub fn artists(&self) -> &ArtistList {
        &self.0.artists
    }

    pub fn artist_detail(&self) -> &ArtistDetail {
        &self.0.artist_detail
    }

    pub fn songs(&self) -> &SongList {
        &self.0.songs
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

    /// The lyrics of what the player plays, looked up while a view wants them.
    pub fn lyrics(&self) -> &lyrics::CurrentLyrics {
        &self.0.lyrics
    }

    /// What the user should hear of from the library, the favorites and the player: those told
    /// since the workspace opened, and those to come.
    pub fn alerts(&self) -> Alerts {
        self.0.alerts.follow()
    }

    /// What `destination` needs as it comes on show: the artists and the songs are read the first
    /// time, the recommendations may be from another day, and the playlists and the favorites are
    /// read again, as another device may have changed them meanwhile.
    pub fn showing(&self, destination: Destination) {
        match destination {
            Destination::Home => self.0.recommendations.renew(),
            Destination::Artists => self.0.artists.open(),
            Destination::Songs => self.0.songs.open(),
            Destination::Playlists => self.0.playlists.refresh(),
            Destination::Favorites => self.0.favorites.refresh(),
            Destination::Albums => {}
        }
    }

    /// Fetches what `destination` shows again: the playlists and the favorites themselves, the
    /// other screens the whole library. Favorites that failed to load are read again from any
    /// screen, as the stars and menus of every screen wait on them.
    pub fn refresh(&self, destination: Destination) {
        match destination {
            Destination::Playlists => self.0.playlists.refresh(),
            Destination::Favorites => self.0.favorites.refresh(),
            Destination::Home | Destination::Artists | Destination::Albums | Destination::Songs => {
                self.0.library.refresh();
            }
        }
        if destination != Destination::Favorites {
            self.0.favorites.reload_if_failed();
        }
    }

    /// Call when the app comes to the front again: the recommendations on show may be from
    /// another day, for an app left open overnight.
    pub fn app_activated(&self) {
        self.0.recommendations.renew();
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

    #[cfg(feature = "desktop")]
    pub(crate) fn keep_media_controls(&self, controls: crate::desktop::WeakControls) {
        lock(&self.0.media_controls).push(controls);
    }

    /// Keeps `routes` going for as long as the workspace lives.
    pub(crate) fn keep(&self, routes: impl IntoIterator<Item = Task<()>>) {
        lock(&self.0.routes).extend(routes);
    }

    /// What a sync brought reaches the search, the home screen's shelves, the artists, the songs
    /// and the recommendations, which may now have something to pick from.
    fn follow_library(&self) -> Task<()> {
        let search = self.0.search.clone();
        let recommendations = self.0.recommendations.clone();
        let shelves = self.0.shelves.clone();
        let artists = self.0.artists.clone();
        let artist_detail = self.0.artist_detail.clone();
        let songs = self.0.songs.clone();
        follow::subscribe(
            &self.0.executor,
            self.0.library.notices(),
            move |notice| match notice {
                LibraryNotice::AlbumsSynced => {
                    shelves.library_synced();
                    recommendations.library_synced();
                    search.library_changed();
                    artists.reload();
                    artist_detail.reload();
                }
                LibraryNotice::ArtistsSynced => {
                    search.library_changed();
                    artists.reload();
                }
                LibraryNotice::SongsSynced => {
                    search.library_changed();
                    songs.reload();
                    artist_detail.songs_synced();
                }
                LibraryNotice::SyncFailed(_)
                | LibraryNotice::SyncFinished
                | LibraryNotice::StageSynced(..) => {}
            },
        )
    }

    /// A play the server has heard of may change what was played and what is worth
    /// recommending.
    fn follow_plays(&self) -> Task<()> {
        let recommendations = self.0.recommendations.clone();
        let shelves = self.0.shelves.clone();
        follow::subscribe(
            &self.0.executor,
            self.0.player.notices(),
            move |notice| match notice {
                PlayerNotice::Played => {
                    shelves.song_played();
                    recommendations.renew();
                }
                PlayerNotice::Skipped => {}
            },
        )
    }

    /// From when the workspace is made, as its sync and the favorites start at once.
    fn collect_alerts(&self) -> [Task<()>; 3] {
        let (executor, collector) = (&self.0.executor, &self.0.alerts);
        [
            alerts::follow_library(executor, &self.0.library, collector),
            alerts::follow_favorites(executor, &self.0.favorites, collector),
            alerts::follow_player(executor, &self.0.player, collector),
        ]
    }

    /// The settings also change for what only their window shows, such as the devices listed,
    /// which the player hears nothing of.
    fn follow_audio_settings(
        &self,
        changes: watch::Receiver<AudioSnapshot>,
        audio: &AudioSnapshot,
    ) -> Task<()> {
        let (mut output, mut transitions) = (audio.output().clone(), audio.transitions());
        let player = self.0.player.clone();
        follow::observe(&self.0.executor, changes, move |audio: AudioSnapshot| {
            if *audio.output() != output {
                output = audio.output().clone();
                player.set_output(output.clone());
            }
            if audio.transitions() != transitions {
                transitions = audio.transitions();
                player.set_transitions(transitions);
            }
        })
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

    fn follow_songs_for_lyrics(&self) -> Task<()> {
        let changes = self.0.player.subscribe();
        let song = changes.borrow().current_song().cloned();
        self.0.lyrics.set_song(song.as_ref());
        let lyrics = self.0.lyrics.clone();
        follow::observe(&self.0.executor, changes, move |shown| {
            lyrics.set_song(shown.current_song());
        })
    }

    pub(crate) fn set_song_cache(&self, song_cache: Option<playback::SongCache>) {
        self.0.player.set_song_cache(song_cache);
    }

    /// Restores the output device as the player found it, and clears the system's media controls.
    pub(crate) fn shutdown(&self) {
        #[cfg(feature = "desktop")]
        {
            let connected: Vec<_> = lock(&self.0.media_controls)
                .iter()
                .filter_map(crate::desktop::WeakControls::upgrade)
                .collect();
            for controls in connected {
                controls.clear();
            }
        }
        self.0.player.shutdown();
    }
}

/// Plays on the output and with the transitions of `audio`, at the volume saved in `store`.
pub(crate) fn new_player(
    executor: &Executor,
    session: Session,
    store: Result<config::Store, config::Error>,
    audio: &AudioSnapshot,
    song_cache: Option<playback::SongCache>,
) -> playback::Player {
    let player = playback::Player::new(
        executor.clone(),
        session,
        store,
        audio.output().clone(),
        song_cache,
    );
    player.set_transitions(audio.transitions());
    player
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
