use account::{SignIn, SignInEvent, Start};
use actions::{
    AddToPlaylist, ChoosePlaylist, EnqueueSongs, NewPlaylist, PlaySongs, SetFavorite, ShowAlbum,
    ShowArtist,
};
use artists::{Artists, ArtistsEvent};
use favorites::{Favorites, FavoritesEvent};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _, TitleBar, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    notification::NotificationType,
    resizable::{ResizableState, h_resizable, resizable_panel},
    tab::{Tab, TabBar},
    v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Div, Entity, FocusHandle, Focusable as _,
    InteractiveElement as _, IntoElement, MouseButton, NavigationDirection, ParentElement as _,
    Pixels, Render, Stateful, Styled as _, Subscription, Task, Window, div,
    prelude::FluentBuilder as _, px,
};
use hiphile_core::navigation::{Destination, History, Location};
use home::Home;
use library::{Library, LibraryEvent, Refresh};
use player::{
    CloseLyrics, CurrentLyrics, CycleRepeat, LyricsView, MiniPlayer, NextSong, Panels, Player,
    PlayerEvent, PreviousSong, QueueView, RepeatMode, SaveQueue, SeekBackward, SeekForward,
    SetRepeat, ToggleLyrics, ToggleMiniPlayer, ToggleMute, TogglePlayback, ToggleQueue,
    ToggleShuffle, VolumeDown, VolumeUp,
};
use playlists::{Playlists, PlaylistsEvent};
use search::{OpenSearch, Search, SearchEvent};
use session::{ReadingsFailed, ReadingsState, Session};
use settings::OpenSettings;
use songs::Songs;
use subsonic::{AlbumId, ArtistId};
use ui::CoverCache;
use updater::{UpdateState, Updater};

use crate::library_activity::{ActivityChanged, LibraryActivity, NoticeChange};
use crate::{
    GoBack, GoForward, ShowAlbums, ShowArtists, ShowFavorites, ShowHome, ShowPlaylists, ShowSongs,
    SignOut,
};

/// When first shown, in rem.
const QUEUE_WIDTH: f32 = 20.;
/// Where the title bar's contents start on macOS, just past the window
/// controls. Those are not scaled with the base font, so this is in pixels.
const MACOS_TITLE_BAR_INSET: Pixels = px(64.);

pub const WORKSPACE_CONTEXT: &str = "Workspace";

/// Routes commands between features and owns no feature state.
pub struct AppShell {
    net: net::Client,
    /// `None` in a build that does not update itself.
    updater: Option<Updater>,
    /// `None` where the system has no credential store; signing in then
    /// always asks for the password.
    keychain: Option<config::Keychain>,
    /// A focus target for the sign-in screen while it has no focused field,
    /// so menu commands still reach this view. Signed in, the screen showing
    /// holds focus instead: a click on empty space then leaves focus where it
    /// was rather than moving it here.
    focus: FocusHandle,
    screen: Screen,
    /// Whether the queue shows beside the library when the window is wide
    /// enough for it.
    queue_shown: bool,
    _readings_failures: Option<Subscription>,
    _song_cache_changes: Subscription,
    _cover_cache_changes: Subscription,
    _system_appearance_changes: Subscription,
    _updater_changes: Option<Task<()>>,
}

enum Screen {
    SignIn {
        view: Entity<SignIn>,
        _subscription: Subscription,
    },
    Workspace(Box<Workspace>),
}

/// Everything that exists only while signed in.
struct Workspace {
    profile: config::Profile,
    destination: Destination,
    history: History<Location>,
    home: Entity<Home>,
    /// Kept while another screen shows, along with its selection and the sync
    /// it runs.
    library: Entity<Library>,
    /// Kept while another screen shows, along with its selection. Loads the
    /// first time it shows.
    artists: Entity<Artists>,
    /// Kept while another screen shows, along with its order and selection.
    /// Loads the first time it shows.
    songs: Entity<Songs>,
    /// Kept while another screen shows: the other screens' menus add to it.
    playlists: Entity<Playlists>,
    /// Kept while another screen shows: the other screens' menus go by it and
    /// add to it.
    favorites: Entity<Favorites>,
    search: Entity<Search>,
    activity: Entity<LibraryActivity>,
    player: Entity<Player>,
    queue: Entity<QueueView>,
    /// Over the screen and the player when open; the rest stay as they were.
    lyrics: Entity<LyricsView>,
    mini_player: Entity<MiniPlayer>,
    panes: Entity<ResizableState>,
    /// Holds the session's decoded covers, so signing out frees them.
    covers: Entity<CoverCache>,
    _player_events: Subscription,
    _library_events: Subscription,
    _artists_events: Subscription,
    _playlists_events: Subscription,
    _favorites_events: Subscription,
    _search_events: Subscription,
    _activity_changes: Subscription,
    _audio_changes: [Subscription; 2],
    _discord_changes: [Subscription; 2],
    _lyrics_changes: [Subscription; 2],
    _queue_changes: Subscription,
    _focus_lost: Subscription,
}

impl Workspace {
    fn location(&self, cx: &App) -> Location {
        match self.destination {
            Destination::Home => Location::Home,
            Destination::Artists => {
                Location::Artists(self.artists.read(cx).selected_artist().cloned())
            }
            Destination::Albums => {
                Location::Albums(self.library.read(cx).selected_album().cloned())
            }
            Destination::Songs => Location::Songs,
            Destination::Playlists => {
                Location::Playlists(self.playlists.read(cx).selected_playlist_id().cloned())
            }
            Destination::Favorites => Location::Favorites,
        }
    }

    /// Gives focus to what is on top, the lyrics or the screen showing, whose
    /// shortcuts then work.
    fn focus_destination(&self, window: &mut Window, cx: &mut App) {
        let focus = match self.destination {
            _ if self.lyrics.read(cx).is_open() => self.lyrics.focus_handle(cx),
            Destination::Home => self.home.focus_handle(cx),
            Destination::Albums => self.library.focus_handle(cx),
            Destination::Artists => self.artists.focus_handle(cx),
            Destination::Songs => self.songs.focus_handle(cx),
            Destination::Playlists => self.playlists.focus_handle(cx),
            Destination::Favorites => self.favorites.focus_handle(cx),
        };
        focus.focus(window, cx);
    }

    /// The narrowest any screen can be right now. Every screen takes it, so
    /// that going from one to another does not open or close the queue.
    fn screen_min_width(&self, window: &Window, cx: &App) -> Pixels {
        self.library
            .read(cx)
            .min_width(window, cx)
            .max(self.artists.read(cx).min_width(window, cx))
            .max(self.playlists.read(cx).min_width(window, cx))
    }
}

/// Says that the dictionaries could not be had, which only Settings would show.
fn tell_readings_failed(error: &str, window: &mut Window, cx: &mut App) {
    let message = format!("{error} Try again in Settings.");
    window.push_notification((NotificationType::Error, message), cx);
}

impl AppShell {
    pub fn new(
        net: net::Client,
        updater: Option<Updater>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        window.set_window_title("hiphile");
        let focus = cx.focus_handle();
        let keychain = config::Keychain::system().ok();
        let screen = Self::sign_in(&net, keychain.clone(), Start::Resume, window, cx);
        let queue_shown = true;
        crate::set_menus(queue_shown, false, false, None, false, RepeatMode::Off, cx);
        let song_cache_changes = cx.subscribe(
            &settings::song_cache_settings(cx),
            |_, song_cache, _: &settings::CacheLimitChanged, cx| {
                let capacity = song_cache.read(cx).capacity();
                player::set_song_capacity(capacity, cx);
            },
        );
        let cover_cache_changes = cx.subscribe(
            &settings::cover_cache_settings(cx),
            |_, cover_cache, _: &settings::CacheLimitChanged, cx| {
                session::set_cover_capacity(cover_cache.read(cx).capacity(), cx);
            },
        );
        let readings_failures = session::readings(cx).map(|readings| {
            // What failed before there was a window to say so in
            if let ReadingsState::Failed(error) = readings.read(cx).state().clone() {
                cx.defer_in(window, move |_, window, cx| {
                    tell_readings_failed(&error, window, cx);
                });
            }
            cx.subscribe_in(
                &readings,
                window,
                |_, _, ReadingsFailed(error), window, cx| {
                    tell_readings_failed(error, window, cx);
                },
            )
        });
        Self {
            net,
            keychain,
            focus,
            screen,
            queue_shown,
            _readings_failures: readings_failures,
            _song_cache_changes: song_cache_changes,
            _cover_cache_changes: cover_cache_changes,
            _system_appearance_changes: settings::follow_system_appearance(window),
            _updater_changes: updater
                .as_ref()
                .map(|updater| gpui_host::notify_on_change(updater.subscribe(), cx)),
            updater,
        }
    }

    fn sign_in(
        net: &net::Client,
        keychain: Option<config::Keychain>,
        start: Start,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Screen {
        let view = cx.new(|cx| SignIn::new(net.clone(), keychain, start, window, cx));
        let subscription = cx.subscribe_in(&view, window, |_, _, event, window, cx| {
            let SignInEvent::SignedIn { session, profile } = event;
            let (session, profile) = (session.clone(), profile.clone());
            // Replacing the screen drops this subscription; do it after the
            // event has been delivered.
            cx.defer_in(window, move |this, window, cx| {
                this.open_workspace(session, profile, window, cx)
            });
        });
        Screen::SignIn {
            view,
            _subscription: subscription,
        }
    }

    fn open_workspace(
        &mut self,
        session: Session,
        profile: config::Profile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let home = cx.new(|cx| Home::new(session.clone(), window, cx));
        let library = cx.new(|cx| Library::new(session.clone(), window, cx));
        let artists = cx.new(|cx| Artists::new(session.clone(), window, cx));
        let songs = cx.new(|cx| Songs::new(session.clone(), window, cx));
        let playlists = cx.new(|cx| Playlists::new(session.clone(), window, cx));
        let favorites = cx.new(|cx| Favorites::new(session.clone(), window, cx));
        let search = cx.new(|cx| Search::new(session.clone(), window, cx));
        let search_events = cx.subscribe_in(&search, window, Self::on_search_event);
        let activity = cx.new(|cx| LibraryActivity::new(session.clone(), cx));
        let activity_changes = cx.subscribe_in(&activity, window, Self::on_library_activity);
        let audio_settings = settings::audio_settings(cx);
        let output = audio_settings.read(cx).output().clone();
        let transitions = audio_settings.read(cx).transitions();
        let discord_settings = settings::discord_settings(cx);
        let cover_key = discord_settings.read(cx).imgbb_key().map(str::to_owned);
        let is_presence_on = discord_settings.read(cx).is_presence_on();
        let shell = cx.weak_entity();
        let player = cx.new(|cx| {
            let mut player = Player::new(session.clone(), output, window, cx);
            player.set_transitions(transitions);
            player.set_cover_key(cover_key.as_deref());
            player.set_presence_on(is_presence_on);
            player.set_panels(move |window, cx| {
                shell
                    .upgrade()
                    .map_or_else(Panels::default, |shell| shell.read(cx).panels(window, cx))
            });
            player
        });
        let queue = cx.new(|cx| QueueView::new(player.clone(), window, cx));
        let (lyrics, mini_player, lyrics_changes) = lyrics_views(&session, &player, window, cx);
        let covers = CoverCache::new(cx);
        // Every screen marks the song the player is on; the shell also
        // re-renders, which updates which commands are available.
        let player_events = cx.subscribe(&player, {
            let home = home.downgrade();
            let library = library.downgrade();
            let artists = artists.downgrade();
            let songs = songs.downgrade();
            let playlists = playlists.downgrade();
            let favorites = favorites.downgrade();
            move |this, _, event, cx| {
                match event {
                    PlayerEvent::NowPlaying { song, playing } => {
                        // With no song the lyrics have nothing to show. Focus
                        // follows once they are gone, as the view holds it.
                        if song.is_none() {
                            this.close_lyrics(cx);
                        }
                        home.update(cx, |home, cx| {
                            home.set_now_playing(song.clone(), *playing, cx)
                        })
                        .ok();
                        library
                            .update(cx, |library, cx| {
                                library.set_now_playing(song.clone(), *playing, cx)
                            })
                            .ok();
                        artists
                            .update(cx, |artists, cx| {
                                artists.set_now_playing(song.clone(), *playing, cx)
                            })
                            .ok();
                        songs
                            .update(cx, |songs, cx| {
                                songs.set_now_playing(song.clone(), *playing, cx)
                            })
                            .ok();
                        playlists
                            .update(cx, |playlists, cx| {
                                playlists.set_now_playing(song.clone(), *playing, cx)
                            })
                            .ok();
                        favorites
                            .update(cx, |favorites, cx| {
                                favorites.set_now_playing(song.clone(), *playing, cx)
                            })
                            .ok();
                    }
                    PlayerEvent::Played => {
                        home.update(cx, |home, cx| home.song_played(cx)).ok();
                    }
                    PlayerEvent::CoverSought { error } => {
                        let error = error
                            .as_ref()
                            .map(|error| format!("Couldn't upload the cover: {error}.").into());
                        settings::discord_settings(cx).update(cx, |settings, cx| {
                            settings.set_upload_error(error, cx);
                        });
                    }
                    PlayerEvent::ShuffleChanged | PlayerEvent::RepeatChanged => {
                        this.update_menus(cx);
                    }
                    PlayerEvent::QueueChanged => {}
                }
                cx.notify();
            }
        });
        let library_events =
            Self::subscribe_to_library(&library, &home, &artists, &songs, &search, window, cx);
        let artists_events = cx.subscribe(&artists, |this, _, event, cx| match event {
            ArtistsEvent::LeftArtist(artist) => {
                this.remember(Location::Artists(Some(artist.clone())), cx);
            }
        });
        let playlists_events = cx.subscribe_in(&playlists, window, Self::on_playlists_event);
        let favorites_events = cx.subscribe_in(&favorites, window, Self::on_favorites_event);
        let audio_changes = follow_audio_settings(&audio_settings, &player, cx);
        let discord_changes = follow_discord_settings(&discord_settings, &player, cx);
        let queue_changes = follow_queue_settings(&player, cx);
        // A focused control that goes away, such as Retry once loading starts,
        // would leave no focus and no working shortcuts.
        let focus_lost = cx.on_focus_lost(window, |this, window, cx| {
            if let Screen::Workspace(workspace) = &this.screen {
                workspace.focus_destination(window, cx);
            }
        });
        let workspace = Workspace {
            profile,
            destination: Destination::Home,
            history: History::new(),
            home,
            library,
            artists,
            songs,
            playlists,
            favorites,
            search,
            activity,
            player,
            queue,
            lyrics,
            mini_player,
            panes: cx.new(|_| ResizableState::default()),
            covers,
            _player_events: player_events,
            _library_events: library_events,
            _artists_events: artists_events,
            _playlists_events: playlists_events,
            _favorites_events: favorites_events,
            _search_events: search_events,
            _activity_changes: activity_changes,
            _audio_changes: audio_changes,
            _discord_changes: discord_changes,
            _lyrics_changes: lyrics_changes,
            _queue_changes: queue_changes,
            _focus_lost: focus_lost,
        };
        workspace.focus_destination(window, cx);
        self.screen = Screen::Workspace(Box::new(workspace));
        self.update_menus(cx);
        cx.notify();
    }

    fn on_search_event(
        &mut self,
        _: &Entity<Search>,
        event: &SearchEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            SearchEvent::ShowArtist(artist) => self.show_artist(artist, window, cx),
            SearchEvent::ShowAlbum(album) => self.show_album(album, window, cx),
            SearchEvent::PlaySong(song) => {
                let songs = [song.clone()];
                self.update_player(window, cx, |player, window, cx| {
                    if !player.play(&songs, 0, cx) {
                        notify_skipped(window, cx);
                    }
                });
            }
        }
    }

    fn on_playlists_event(
        &mut self,
        _: &Entity<Playlists>,
        event: &PlaylistsEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            PlaylistsEvent::CreatedEmpty => {
                self.show_destination(Destination::Playlists, window, cx);
            }
            PlaylistsEvent::ChooserChanged => cx.notify(),
            PlaylistsEvent::LeftPlaylist(playlist) => {
                self.remember(Location::Playlists(Some(playlist.clone())), cx);
            }
        }
    }

    fn on_favorites_event(
        &mut self,
        _: &Entity<Favorites>,
        event: &FavoritesEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let FavoritesEvent::LoadFailed(error) = event;
        // The favorites' own screen says so where the list would be
        if !self.is_showing_favorites() {
            let message =
                format!("Couldn't load favorites. {error} Choose Refresh Library to try again.");
            window.push_notification((NotificationType::Error, message), cx);
        }
    }

    fn is_showing_favorites(&self) -> bool {
        matches!(
            &self.screen,
            Screen::Workspace(workspace) if matches!(workspace.destination, Destination::Favorites)
        )
    }

    fn on_library_activity(
        &mut self,
        activity: &Entity<LibraryActivity>,
        _: &ActivityChanged,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match activity.update(cx, |it, _| it.notice_change()) {
            Some(NoticeChange::Show) => {
                window.push_notification(LibraryActivity::notice(activity), cx);
            }
            Some(NoticeChange::Hide) => window.remove_notification::<LibraryActivity>(cx),
            None => {}
        }
    }

    fn subscribe_to_library(
        library: &Entity<Library>,
        home: &Entity<Home>,
        artists: &Entity<Artists>,
        songs: &Entity<Songs>,
        search: &Entity<Search>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(library, window, {
            let home = home.downgrade();
            let artists = artists.downgrade();
            let songs = songs.downgrade();
            let search = search.downgrade();
            move |this, _, event, window, cx| match event {
                LibraryEvent::SyncProgress {
                    stage,
                    saved,
                    total,
                } => {
                    if let Screen::Workspace(workspace) = &this.screen {
                        workspace.activity.update(cx, |it, cx| {
                            it.sync_progress(*stage, *saved, *total, cx);
                        });
                    }
                }
                LibraryEvent::SyncFinished => {
                    if let Screen::Workspace(workspace) = &this.screen {
                        workspace.activity.update(cx, |it, cx| it.sync_finished(cx));
                    }
                }
                LibraryEvent::AlbumsSynced => {
                    home.update(cx, |home, cx| home.library_synced(cx)).ok();
                    artists.update(cx, |artists, cx| artists.refresh(cx)).ok();
                    search.update(cx, |it, cx| it.library_changed(cx)).ok();
                }
                LibraryEvent::ArtistsSynced => {
                    artists
                        .update(cx, |artists, cx| artists.artists_synced(cx))
                        .ok();
                    search.update(cx, |it, cx| it.library_changed(cx)).ok();
                }
                LibraryEvent::SongsSynced => {
                    songs.update(cx, |songs, cx| songs.songs_synced(cx)).ok();
                    artists
                        .update(cx, |artists, cx| artists.songs_synced(cx))
                        .ok();
                    search.update(cx, |it, cx| it.library_changed(cx)).ok();
                }
                LibraryEvent::SyncFailed(error) => {
                    // The albums' screen says so itself
                    if let Screen::Workspace(workspace) = &this.screen
                        && workspace.destination != Destination::Albums
                    {
                        let message = format!("Couldn't refresh the library. {error}");
                        window.push_notification((NotificationType::Error, message), cx);
                    }
                }
                LibraryEvent::LeftAlbum(album) => {
                    this.remember(Location::Albums(Some(album.clone())), cx);
                }
            }
        })
    }

    fn destination(&self) -> Option<Destination> {
        match &self.screen {
            Screen::Workspace(workspace) => Some(workspace.destination),
            Screen::SignIn { .. } => None,
        }
    }

    fn update_menus(&self, cx: &mut App) {
        let (shuffled, repeat) = self.player().map_or((false, RepeatMode::Off), |player| {
            let player = player.read(cx);
            (player.is_shuffled(), player.repeat())
        });
        let is_lyrics_open = self.is_lyrics_open(cx);
        crate::set_menus(
            self.queue_shown && !is_lyrics_open,
            is_lyrics_open,
            self.is_mini_player_open(cx),
            self.destination(),
            shuffled,
            repeat,
            cx,
        );
    }

    /// Goes to `destination`; going back returns to where this leaves.
    fn show_destination(
        &mut self,
        destination: Destination,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_lyrics(cx);
        let Screen::Workspace(workspace) = &mut self.screen else {
            return;
        };
        if workspace.destination != destination {
            let left = workspace.location(cx);
            workspace.history.visit(left);
            self.enter_destination(destination, window, cx);
        }
    }

    fn enter_destination(
        &mut self,
        destination: Destination,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Screen::Workspace(workspace) = &mut self.screen else {
            return;
        };
        if workspace.destination == destination {
            return;
        }
        workspace.destination = destination;
        match destination {
            Destination::Home => workspace
                .home
                .update(cx, |home, cx| home.renew_recommended(cx)),
            // Another device may have changed them meanwhile
            Destination::Playlists => workspace
                .playlists
                .update(cx, |playlists, cx| playlists.refresh(window, cx)),
            Destination::Favorites => workspace
                .favorites
                .update(cx, |favorites, cx| favorites.refresh(cx)),
            Destination::Artists => workspace
                .artists
                .update(cx, |artists, cx| artists.activate(cx)),
            Destination::Songs => workspace.songs.update(cx, |songs, cx| songs.activate(cx)),
            Destination::Albums => {}
        }
        workspace.focus_destination(window, cx);
        self.update_menus(cx);
        cx.notify();
    }

    fn show_artist(&mut self, artist: &ArtistId, window: &mut Window, cx: &mut Context<Self>) {
        self.leave_for(&Location::Artists(Some(artist.clone())), cx);
        self.go_to(Location::Artists(Some(artist.clone())), window, cx);
    }

    fn show_album(&mut self, album: &AlbumId, window: &mut Window, cx: &mut Context<Self>) {
        self.leave_for(&Location::Albums(Some(album.clone())), cx);
        self.go_to(Location::Albums(Some(album.clone())), window, cx);
    }

    fn leave_for(&mut self, next: &Location, cx: &mut Context<Self>) {
        if let Screen::Workspace(workspace) = &mut self.screen {
            let here = workspace.location(cx);
            if here.is_worth_returning_to_from(next) {
                workspace.history.visit(here);
            }
        }
    }

    /// Records that a screen moved on from `left` by itself.
    fn remember(&mut self, left: Location, cx: &mut Context<Self>) {
        if let Screen::Workspace(workspace) = &mut self.screen {
            workspace.history.visit(left);
            cx.notify();
        }
    }

    /// Shows `location`, leaving the history as it is.
    fn go_to(&mut self, location: Location, window: &mut Window, cx: &mut Context<Self>) {
        self.close_lyrics(cx);
        self.enter_destination(location.destination(), window, cx);
        let Screen::Workspace(workspace) = &self.screen else {
            return;
        };
        match location {
            Location::Artists(Some(artist)) => workspace
                .artists
                .update(cx, |artists, cx| artists.reveal_artist(artist, window, cx)),
            Location::Albums(Some(album)) => workspace
                .library
                .update(cx, |library, cx| library.reveal_album(album, window, cx)),
            Location::Playlists(Some(playlist)) => {
                workspace.playlists.update(cx, |playlists, cx| {
                    playlists.reveal_playlist(playlist, window, cx)
                })
            }
            Location::Home
            | Location::Artists(None)
            | Location::Albums(None)
            | Location::Songs
            | Location::Playlists(None)
            | Location::Favorites => {}
        }
        cx.notify();
    }

    /// Goes back from the lyrics if they are open, and otherwise to the place
    /// before this one.
    fn go_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_lyrics_open(cx) {
            self.set_lyrics_open(false, window, cx);
            return;
        }
        let Screen::Workspace(workspace) = &mut self.screen else {
            return;
        };
        let here = workspace.location(cx);
        if let Some(previous) = workspace.history.back(here) {
            self.go_to(previous, window, cx);
        }
    }

    fn go_forward(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_lyrics_open(cx) {
            return;
        }
        let Screen::Workspace(workspace) = &mut self.screen else {
            return;
        };
        let here = workspace.location(cx);
        if let Some(next) = workspace.history.forward(here) {
            self.go_to(next, window, cx);
        }
    }

    /// Whether going back and going forward lead anywhere.
    fn history_available(&self, cx: &App) -> (bool, bool) {
        match &self.screen {
            Screen::Workspace(_) if self.is_lyrics_open(cx) => (true, false),
            Screen::Workspace(workspace) => (
                workspace.history.can_go_back(),
                workspace.history.can_go_forward(),
            ),
            Screen::SignIn { .. } => (false, false),
        }
    }

    /// Fetches what the screen showing is about again, for the screens that
    /// do not run it themselves.
    fn refresh(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Screen::Workspace(workspace) = &self.screen else {
            return;
        };
        match workspace.destination {
            Destination::Playlists => workspace
                .playlists
                .update(cx, |playlists, cx| playlists.refresh(window, cx)),
            Destination::Favorites => workspace
                .favorites
                .update(cx, |favorites, cx| favorites.refresh(cx)),
            _ => workspace
                .library
                .update(cx, |library, cx| library.refresh_library(cx)),
        }
    }

    /// Tries favorites that failed to load again from any screen, whose stars
    /// and menus are waiting on them. The favorites' own screen reads them
    /// anyway.
    fn retry_favorites(&self, cx: &mut Context<Self>) {
        if let Screen::Workspace(workspace) = &self.screen
            && !self.is_showing_favorites()
        {
            workspace
                .favorites
                .update(cx, |favorites, cx| favorites.reload_if_failed(cx));
        }
    }

    fn update_playlists(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
        update: impl FnOnce(&mut Playlists, &mut Window, &mut Context<Playlists>),
    ) {
        if let Screen::Workspace(workspace) = &self.screen {
            workspace
                .playlists
                .update(cx, |playlists, cx| update(playlists, window, cx));
        }
    }

    /// Drops the workspace, which stops playback, forgets the saved
    /// password, and returns to signing in.
    fn sign_out(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Screen::Workspace(workspace) = &self.screen else {
            return;
        };
        if let Some(keychain) = self.keychain.clone() {
            let profile = workspace.profile.clone();
            cx.background_spawn(async move { keychain.delete_password(&profile) })
                .detach();
        }
        // The window keeps its notices, and this one would never hear the end
        window.remove_notification::<LibraryActivity>(cx);
        self.focus.focus(window, cx);
        self.screen = Self::sign_in(&self.net, self.keychain.clone(), Start::Fresh, window, cx);
        self.update_menus(cx);
        cx.notify();
    }

    fn player(&self) -> Option<&Entity<Player>> {
        match &self.screen {
            Screen::Workspace(workspace) => Some(&workspace.player),
            Screen::SignIn { .. } => None,
        }
    }

    fn play_songs(&mut self, action: &PlaySongs, window: &mut Window, cx: &mut Context<Self>) {
        self.update_player(window, cx, |player, window, cx| {
            let is_queued = match action.start_ix() {
                Some(start_ix) => player.play(action.songs(), start_ix, cx),
                None => player.play_whole(action.songs(), cx),
            };
            if !is_queued {
                notify_skipped(window, cx);
            }
        });
    }

    fn enqueue_songs(
        &mut self,
        action: &EnqueueSongs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let placement = match action.placement() {
            actions::Placement::Next => player::Placement::Next,
            actions::Placement::Last => player::Placement::Last,
        };
        self.update_player(window, cx, |player, window, cx| {
            if !player.enqueue(action.songs(), placement, cx) {
                notify_skipped(window, cx);
            }
        });
    }

    fn update_player(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
        update: impl FnOnce(&mut Player, &mut Window, &mut Context<Player>),
    ) {
        if let Some(player) = self.player() {
            player.update(cx, |player, cx| update(player, window, cx));
        }
    }

    fn is_lyrics_open(&self, cx: &App) -> bool {
        match &self.screen {
            Screen::Workspace(workspace) => workspace.lyrics.read(cx).is_open(),
            Screen::SignIn { .. } => false,
        }
    }

    fn is_mini_player_open(&self, cx: &App) -> bool {
        match &self.screen {
            Screen::Workspace(workspace) => workspace.mini_player.read(cx).is_open(),
            Screen::SignIn { .. } => false,
        }
    }

    fn set_lyrics_open(&mut self, is_open: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Screen::Workspace(workspace) = &self.screen else {
            return;
        };
        workspace
            .lyrics
            .update(cx, |lyrics, cx| lyrics.set_open(is_open, cx));
        workspace.focus_destination(window, cx);
        self.update_menus(cx);
        cx.notify();
    }

    /// Closes the lyrics from where there is no window to move focus in. The
    /// callers go on to focus something, or the view goes and gives up focus.
    fn close_lyrics(&mut self, cx: &mut Context<Self>) {
        if !self.is_lyrics_open(cx) {
            return;
        }
        if let Screen::Workspace(workspace) = &self.screen {
            workspace
                .lyrics
                .update(cx, |lyrics, cx| lyrics.set_open(false, cx));
        }
        self.update_menus(cx);
        cx.notify();
    }

    /// With the lyrics over it, the queue is not in view either way, so going back from them shows it.
    fn toggle_queue(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_lyrics_open(cx) {
            self.queue_shown = true;
            self.set_lyrics_open(false, window, cx);
            return;
        }
        self.queue_shown = !self.queue_shown;
        self.update_menus(cx);
        cx.notify();
    }

    /// Whether the window is wide enough for the queue beside the library
    /// at the widths their panes have now. The queue keeps its width as the
    /// window narrows, so past this point it closes instead of pushing the
    /// library out of the window.
    fn queue_fits(&self, window: &Window, cx: &App) -> bool {
        let Screen::Workspace(workspace) = &self.screen else {
            return false;
        };
        let queue = workspace
            .panes
            .read(cx)
            .sizes()
            .get(1)
            .copied()
            .filter(|width| !gpui_kit::IsZero::is_zero(width))
            .unwrap_or(window.rem_size() * QUEUE_WIDTH);
        window.viewport_size().width >= workspace.screen_min_width(window, cx) + queue
    }

    fn panels(&self, window: &Window, cx: &App) -> Panels {
        Panels {
            is_lyrics_open: self.is_lyrics_open(cx),
            is_mini_player_open: self.is_mini_player_open(cx),
            is_queue_shown: self.queue_shown,
            queue_fits: self.queue_fits(window, cx),
        }
    }

    fn handle_navigation(&self, root: Stateful<Div>, cx: &mut Context<Self>) -> Stateful<Div> {
        let (can_go_back, can_go_forward) = self.history_available(cx);
        root.on_mouse_down(
            MouseButton::Navigate(NavigationDirection::Back),
            cx.listener(|this, _, window, cx| this.go_back(window, cx)),
        )
        .on_mouse_down(
            MouseButton::Navigate(NavigationDirection::Forward),
            cx.listener(|this, _, window, cx| this.go_forward(window, cx)),
        )
        .when(can_go_back, |this| {
            this.on_action(cx.listener(|this, _: &GoBack, window, cx| this.go_back(window, cx)))
        })
        .when(can_go_forward, |this| {
            this.on_action(
                cx.listener(|this, _: &GoForward, window, cx| this.go_forward(window, cx)),
            )
        })
    }

    fn handle_song_commands(&self, root: Stateful<Div>, cx: &mut Context<Self>) -> Stateful<Div> {
        let lyrics_open = self.is_lyrics_open(cx);
        let has_song = self
            .player()
            .is_some_and(|player| player.read(cx).has_song());
        root.when(lyrics_open, |this| {
            this.on_action(cx.listener(|this, _: &CloseLyrics, window, cx| {
                this.set_lyrics_open(false, window, cx);
            }))
        })
        .when(has_song, |this| {
            this.on_action(cx.listener(|this, _: &ToggleLyrics, window, cx| {
                let is_open = this.is_lyrics_open(cx);
                this.set_lyrics_open(!is_open, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleMiniPlayer, window, cx| {
                if let Screen::Workspace(workspace) = &this.screen {
                    workspace
                        .mini_player
                        .update(cx, |mini_player, cx| mini_player.toggle(window, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &TogglePlayback, window, cx| {
                this.update_player(window, cx, |player, _, cx| player.toggle_playback(cx));
            }))
            .on_action(cx.listener(|this, _: &NextSong, window, cx| {
                this.update_player(window, cx, |player, _, cx| player.next(cx));
            }))
            .on_action(cx.listener(|this, _: &PreviousSong, window, cx| {
                this.update_player(window, cx, |player, _, cx| player.previous(cx));
            }))
            .on_action(cx.listener(|this, _: &SeekForward, window, cx| {
                this.update_player(window, cx, Player::seek_forward);
            }))
            .on_action(cx.listener(|this, _: &SeekBackward, window, cx| {
                this.update_player(window, cx, Player::seek_backward);
            }))
        })
    }

    fn render_under_title_bar(
        &self,
        content: AnyElement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let lyrics = match &self.screen {
            Screen::Workspace(workspace) => Some(&workspace.lyrics),
            Screen::SignIn { .. } => None,
        };
        let reveal = lyrics.map(|lyrics| lyrics.read(cx).reveal(cx));
        // What is under the lyrics fades with them, so it needs the frames too
        if reveal.is_some_and(|reveal| reveal.is_moving) {
            window.request_animation_frame();
        }
        let under_opacity = reveal.map_or(1., |reveal| reveal.under_opacity);
        let is_covered = reveal.is_some_and(|reveal| reveal.is_covering);
        let lyrics = lyrics
            .filter(|_| reveal.is_some_and(|reveal| reveal.is_shown))
            .cloned();
        div()
            .relative()
            .flex_1()
            .min_h_0()
            // Under the lyrics only while they are on their way, so that nothing
            // out of sight takes Tab or a frame.
            .when(!is_covered, |this| {
                this.child(
                    v_flex()
                        .size_full()
                        .opacity(under_opacity)
                        .child(div().flex_1().min_h_0().child(content))
                        .when_some(self.player().cloned(), |this, player| this.child(player)),
                )
            })
            .children(lyrics)
    }

    fn render_workspace(
        &self,
        workspace: &Workspace,
        queue_visible: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rem = window.rem_size();
        let screen_min = workspace.screen_min_width(window, cx);
        h_resizable("workspace-panes")
            .with_state(&workspace.panes)
            .child(resizable_panel().size_range(screen_min..rem * 240.).child(
                match workspace.destination {
                    Destination::Home => workspace.home.clone().into_any_element(),
                    Destination::Albums => workspace.library.clone().into_any_element(),
                    Destination::Artists => workspace.artists.clone().into_any_element(),
                    Destination::Songs => workspace.songs.clone().into_any_element(),
                    Destination::Playlists => workspace.playlists.clone().into_any_element(),
                    Destination::Favorites => workspace.favorites.clone().into_any_element(),
                },
            ))
            .child(
                resizable_panel()
                    .size(rem * QUEUE_WIDTH)
                    .size_range(rem * 16. ..rem * 28.)
                    .fixed()
                    .visible(queue_visible)
                    .child(workspace.queue.clone()),
            )
            .into_any_element()
    }

    fn render_title_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let signed_in = matches!(self.screen, Screen::Workspace(_));
        let destination_ix = self
            .destination()
            .and_then(|current| Destination::ALL.iter().position(|d| *d == current));
        let (can_go_back, can_go_forward) = self.history_available(cx);
        let update_ready = self
            .updater
            .as_ref()
            .is_some_and(|updater| matches!(updater.status().state(), UpdateState::Ready(_)));
        TitleBar::new()
            .when(cfg!(target_os = "macos"), |this| {
                this.pl(MACOS_TITLE_BAR_INSET)
            })
            .child(
                h_flex()
                    .size_full()
                    .px_1()
                    .pr_3()
                    .gap_2()
                    .when(signed_in, |this| {
                        this.child(
                            title_bar_controls()
                                .child(
                                    Button::new("go-back")
                                        .ghost()
                                        .small()
                                        .icon(IconName::ChevronLeft)
                                        .disabled(!can_go_back)
                                        .accessibility_label("Back")
                                        .tooltip_with_action("Back", &GoBack, None)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.go_back(window, cx);
                                        })),
                                )
                                .child(
                                    Button::new("go-forward")
                                        .ghost()
                                        .small()
                                        .icon(IconName::ChevronRight)
                                        .disabled(!can_go_forward)
                                        .accessibility_label("Forward")
                                        .tooltip_with_action("Forward", &GoForward, None)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.go_forward(window, cx);
                                        })),
                                ),
                        )
                        .child(div().text_sm().font_medium().child("Hiphile"))
                        .child(
                            div().flex_none().ml_3().child(
                                TabBar::new("destinations")
                                    .segmented()
                                    .small()
                                    .selected_index(destination_ix.unwrap_or_default())
                                    .on_click(cx.listener(|this, ix: &usize, window, cx| {
                                        if let Some(destination) = Destination::ALL.get(*ix) {
                                            this.show_destination(*destination, window, cx);
                                        }
                                    }))
                                    .children(
                                        Destination::ALL.map(|destination| {
                                            Tab::new().label(destination.label())
                                        }),
                                    ),
                            ),
                        )
                    })
                    .child(div().flex_1())
                    .when(update_ready, |this| {
                        this.child(
                            title_bar_controls().child(
                                Button::new("restart-to-update")
                                    .outline()
                                    .small()
                                    .label("Restart to update")
                                    .on_click(|_, _, cx| cx.restart()),
                            ),
                        )
                    })
                    .child(
                        title_bar_controls().child(
                            Button::new("open-settings")
                                .ghost()
                                .small()
                                .icon(IconName::Settings)
                                .accessibility_label("Settings")
                                .tooltip_with_action("Settings", &OpenSettings, None)
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(OpenSettings), cx);
                                }),
                        ),
                    ),
            )
            .into_any_element()
    }
}

fn follow_audio_settings(
    audio_settings: &Entity<settings::AudioSettings>,
    player: &Entity<Player>,
    cx: &mut Context<AppShell>,
) -> [Subscription; 2] {
    let output_changes = cx.subscribe(audio_settings, {
        let player = player.downgrade();
        move |_, audio_settings, _: &settings::OutputChanged, cx| {
            let output = audio_settings.read(cx).output().clone();
            player
                .update(cx, |player, cx| player.set_output(output, cx))
                .ok();
        }
    });
    let transitions_changes = cx.subscribe(audio_settings, {
        let player = player.downgrade();
        move |_, audio_settings, _: &settings::TransitionsChanged, cx| {
            let transitions = audio_settings.read(cx).transitions();
            player
                .update(cx, |player, _| player.set_transitions(transitions))
                .ok();
        }
    });
    [output_changes, transitions_changes]
}

fn follow_queue_settings(player: &Entity<Player>, cx: &mut Context<AppShell>) -> Subscription {
    let queue_settings = settings::queue_settings(cx);
    actions::set_skip_instrumentals(queue_settings.read(cx).is_skip_instrumentals_on(), cx);
    cx.subscribe(&queue_settings, {
        let player = player.downgrade();
        move |_, queue_settings, _: &settings::SkipInstrumentalsChanged, cx| {
            let is_on = queue_settings.read(cx).is_skip_instrumentals_on();
            actions::set_skip_instrumentals(is_on, cx);
            player
                .update(cx, |player, cx| player.apply_skipping(cx))
                .ok();
        }
    })
}

fn notify_skipped(window: &mut Window, cx: &mut App) {
    let message =
        "Instrumentals are being skipped. Turn off Skip instrumentals in Settings to play them.";
    window.push_notification((NotificationType::Info, message), cx);
}

fn follow_discord_settings(
    discord_settings: &Entity<settings::DiscordSettings>,
    player: &Entity<Player>,
    cx: &mut Context<AppShell>,
) -> [Subscription; 2] {
    let key_changes = cx.subscribe(discord_settings, {
        let player = player.downgrade();
        move |_, discord_settings, _: &settings::ImgbbKeyChanged, cx| {
            let key = discord_settings.read(cx).imgbb_key().map(str::to_owned);
            player
                .update(cx, |player, _| player.set_cover_key(key.as_deref()))
                .ok();
        }
    });
    let presence_changes = cx.subscribe(discord_settings, {
        let player = player.downgrade();
        move |_, discord_settings, _: &settings::PresenceChanged, cx| {
            let is_on = discord_settings.read(cx).is_presence_on();
            player
                .update(cx, |player, _| player.set_presence_on(is_on))
                .ok();
        }
    });
    [key_changes, presence_changes]
}

/// The title bar moves the window, and on Windows a press that goes on to the title bar starts
/// that move, which takes the release and so the click.
fn title_bar_controls() -> Div {
    h_flex()
        .flex_none()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

/// What shows the lyrics of what `player` plays, kept on the chosen highlight for
/// as long as the subscriptions live.
fn lyrics_views(
    session: &Session,
    player: &Entity<Player>,
    window: &Window,
    cx: &mut Context<AppShell>,
) -> (Entity<LyricsView>, Entity<MiniPlayer>, [Subscription; 2]) {
    let finder = lyrics::Finder::user(session.client().net().clone());
    let lyrics_settings = settings::lyrics_settings(cx);
    let highlight = lyrics_settings.read(cx).highlight();
    let current = cx.new(|cx| {
        let mut current = CurrentLyrics::new(player.clone(), finder);
        current.set_highlight(highlight, cx);
        current
    });
    let highlight_changes = cx.subscribe(&lyrics_settings, {
        let current = current.downgrade();
        move |_, lyrics_settings, _: &settings::LyricsHighlightChanged, cx| {
            let highlight = lyrics_settings.read(cx).highlight();
            current
                .update(cx, |current, cx| current.set_highlight(highlight, cx))
                .ok();
        }
    });
    let view = cx.new(|cx| LyricsView::new(player.clone(), current.clone(), cx));
    let mini_player = cx.new(|cx| MiniPlayer::new(player.clone(), current, window, cx));
    let mini_player_changes = cx.observe(&mini_player, |this, _, cx| {
        this.update_menus(cx);
        cx.notify();
    });
    (view, mini_player, [highlight_changes, mini_player_changes])
}

impl Render for AppShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let queue_fits = self.queue_fits(window, cx);
        let queue_visible = self.queue_shown && queue_fits;
        let content: AnyElement = match &self.screen {
            Screen::SignIn { view, .. } => view.clone().into_any_element(),
            Screen::Workspace(workspace) => {
                self.render_workspace(workspace, queue_visible, window, cx)
            }
        };
        let covers = match &self.screen {
            Screen::Workspace(workspace) => Some(workspace.covers.clone()),
            Screen::SignIn { .. } => None,
        };
        let signed_in = matches!(self.screen, Screen::Workspace(_));
        let has_queue = self
            .player()
            .is_some_and(|player| player.read(cx).has_queued_songs());
        let chooser = match &self.screen {
            Screen::Workspace(workspace) => workspace.playlists.read(cx).render_chooser(window),
            Screen::SignIn { .. } => None,
        };
        // Commands are registered only where they do something, so menus and
        // shortcuts are disabled otherwise.
        v_flex()
            .when_some(covers, |this, covers| this.image_cache(covers))
            .id("app-shell")
            .key_context(WORKSPACE_CONTEXT)
            .when(!signed_in, |this| this.track_focus(&self.focus))
            .size_full()
            .min_h_0()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .when(signed_in, |this| {
                this.on_action(
                    cx.listener(|this, _: &SignOut, window, cx| this.sign_out(window, cx)),
                )
                .on_action(cx.listener(|this, _: &ShowHome, window, cx| {
                    this.show_destination(Destination::Home, window, cx);
                }))
                .on_action(cx.listener(|this, _: &ShowAlbums, window, cx| {
                    this.show_destination(Destination::Albums, window, cx);
                }))
                .on_action(cx.listener(|this, _: &ShowArtists, window, cx| {
                    this.show_destination(Destination::Artists, window, cx);
                }))
                .on_action(cx.listener(|this, _: &OpenSearch, window, cx| {
                    if let Screen::Workspace(workspace) = &this.screen {
                        workspace
                            .search
                            .update(cx, |search, cx| search.open(window, cx));
                    }
                }))
                .on_action(cx.listener(|this, _: &ShowSongs, window, cx| {
                    this.show_destination(Destination::Songs, window, cx);
                }))
                .on_action(cx.listener(|this, _: &ShowPlaylists, window, cx| {
                    this.show_destination(Destination::Playlists, window, cx);
                }))
                .on_action(cx.listener(|this, _: &ShowFavorites, window, cx| {
                    this.show_destination(Destination::Favorites, window, cx);
                }))
                .on_action(cx.listener(|this, action: &SetFavorite, window, cx| {
                    if let Screen::Workspace(workspace) = &this.screen {
                        workspace.favorites.update(cx, |favorites, cx| {
                            let favorite = action.favorite().clone();
                            favorites.set_favorite(favorite, action.is_favorite(), window, cx);
                        });
                    }
                }))
                .on_action(cx.listener(|this, action: &ShowAlbum, window, cx| {
                    this.show_album(action.album(), window, cx);
                }))
                .on_action(cx.listener(|this, action: &ShowArtist, window, cx| {
                    this.show_artist(action.artist(), window, cx);
                }))
                // On the way in, as the albums' screen runs Refresh itself
                .capture_action(cx.listener(|this, _: &Refresh, _, cx| this.retry_favorites(cx)))
                .on_action(cx.listener(|this, _: &Refresh, window, cx| this.refresh(window, cx)))
                .on_action(cx.listener(|this, action: &AddToPlaylist, window, cx| {
                    this.update_playlists(window, cx, |playlists, window, cx| {
                        playlists.add_songs(action.playlist(), action.songs(), window, cx);
                    });
                }))
                .on_action(cx.listener(|this, action: &NewPlaylist, window, cx| {
                    this.update_playlists(window, cx, |playlists, window, cx| {
                        playlists.open_new_playlist(action.songs().clone(), window, cx);
                    });
                }))
                .on_action(cx.listener(|this, action: &ChoosePlaylist, window, cx| {
                    this.update_playlists(window, cx, |playlists, window, cx| {
                        playlists.open_chooser(action.songs().clone(), window, cx);
                    });
                }))
                .on_action(cx.listener(Self::play_songs))
                .on_action(cx.listener(Self::enqueue_songs))
                .on_action(cx.listener(|this, _: &VolumeUp, window, cx| {
                    this.update_player(window, cx, Player::volume_up);
                }))
                .on_action(cx.listener(|this, _: &VolumeDown, window, cx| {
                    this.update_player(window, cx, Player::volume_down);
                }))
                .on_action(cx.listener(|this, _: &ToggleMute, window, cx| {
                    this.update_player(window, cx, |player, _, cx| player.toggle_mute(cx));
                }))
                .on_action(cx.listener(|this, _: &ToggleShuffle, window, cx| {
                    this.update_player(window, cx, |player, _, cx| player.toggle_shuffle(cx));
                }))
                .on_action(cx.listener(|this, _: &CycleRepeat, window, cx| {
                    this.update_player(window, cx, |player, _, cx| player.cycle_repeat(cx));
                }))
                .on_action(cx.listener(|this, action: &SetRepeat, window, cx| {
                    this.update_player(window, cx, |player, _, cx| {
                        player.set_repeat(action.repeat(), cx)
                    });
                }))
            })
            .when(queue_fits, |this| {
                this.on_action(cx.listener(|this, _: &ToggleQueue, window, cx| {
                    this.toggle_queue(window, cx);
                }))
            })
            .when(has_queue, |this| {
                this.on_action(cx.listener(|this, _: &SaveQueue, window, cx| {
                    let Some(songs) = this.player().map(|player| player.read(cx).queued_songs())
                    else {
                        return;
                    };
                    this.update_playlists(window, cx, |playlists, window, cx| {
                        playlists.open_new_playlist(songs, window, cx);
                    });
                }))
            })
            .when(signed_in, |this| self.handle_song_commands(this, cx))
            .when(signed_in, |this| self.handle_navigation(this, cx))
            .child(self.render_title_bar(cx))
            .child(self.render_under_title_bar(content, window, cx))
            .children(chooser)
    }
}

#[cfg(test)]
mod tests;
