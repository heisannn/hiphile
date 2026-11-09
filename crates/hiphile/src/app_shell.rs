use account::{AddServer, SignIn, SignOut, SwitchServer};
use actions::{
    AddToPlaylist, ChoosePlaylist, EnqueueSongs, NewPlaylist, PlaySongs, ShowAlbum, ShowArtist,
    ToggleFavoriteOf,
};
use artists::{Artists, ArtistsEvent};
use favorites::Favorites;
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
    Pixels, Render, SharedString, Stateful, Styled as _, Subscription, Task, WeakEntity, Window,
    div, prelude::FluentBuilder as _, px,
};
use hiphile_core::alerts::{Alert, AlertKind, Alerts};
use hiphile_core::desktop::{DiscordStatus, MediaNotice, SystemMediaControls};
use hiphile_core::favorites::FavoritesState;
use hiphile_core::library::LibraryActivity;
use hiphile_core::navigation::{Destination, History, Location};
use hiphile_core::playlists::PlaylistsState;
use hiphile_core::settings::QueueSettings;
use hiphile_core::{Hiphile, HiphileSnapshot, SwitchFailed, Workspace};
use home::Home;
use library::{Library, LibraryEvent};
use playback::PlayerSnapshot;
use player::{
    CloseLyrics, CycleRepeat, LyricsView, MiniPlayer, NextSong, Panels, Player, PreviousSong,
    QueueView, RepeatMode, SaveQueue, SeekBackward, SeekForward, SetRepeat, ToggleLyrics,
    ToggleMiniPlayer, ToggleMute, TogglePlayback, ToggleQueue, ToggleShuffle, VolumeDown, VolumeUp,
};
use playlists::{Playlists, PlaylistsEvent};
use presence::Presence;
use search::{OpenSearch, Search, SearchEvent};
use settings::OpenSettings;
use songs::Songs;
use subsonic::{AlbumId, ArtistId, SongId};
use ui::CoverCache;
use updater::{UpdateState, Updater};

use crate::library_activity::ActivityNotice;
use crate::{
    GoBack, GoForward, MenuState, Refresh, ShowAlbums, ShowArtists, ShowFavorites, ShowHome,
    ShowPlaylists, ShowSongs,
};

/// When first shown, in rem.
const QUEUE_WIDTH: f32 = 20.;
/// Where the title bar's contents start on macOS, just past the window
/// controls. Those are not scaled with the base font, so this is in pixels.
const MACOS_TITLE_BAR_INSET: Pixels = px(64.);

pub const WORKSPACE_CONTEXT: &str = "Workspace";

/// Routes commands between features and owns no feature state.
pub struct AppShell {
    hiphile: Hiphile,
    /// `None` in a build that does not update itself.
    updater: Option<Updater>,
    /// A focus target for the sign-in screen while it has no focused field,
    /// so menu commands still reach this view. Signed in, the screen showing
    /// holds focus instead: a click on empty space then leaves focus where it
    /// was rather than moving it here.
    focus: FocusHandle,
    screen: Screen,
    /// Whether the queue shows beside the library when the window is wide
    /// enough for it.
    queue_shown: bool,
    _alerts: Task<()>,
    _signing_in: Task<()>,
    _switching: Task<()>,
    _system_appearance_changes: Subscription,
    _updater_changes: Option<Task<()>>,
}

enum Screen {
    SignIn(Entity<SignIn>),
    Workspace(Box<WorkspaceScreen>),
}

/// The views of the workspace, which exist only while signed in.
struct WorkspaceScreen {
    core: Workspace,
    destination: Destination,
    history: History<Location>,
    home: Entity<Home>,
    /// Kept while another screen shows, along with its selection.
    library: Entity<Library>,
    /// Kept while another screen shows, along with its selection. Loads the
    /// first time it shows.
    artists: Entity<Artists>,
    /// Kept while another screen shows, along with its order and selection.
    /// Loads the first time it shows.
    songs: Entity<Songs>,
    /// Kept while another screen shows: the other screens' menus add to it.
    playlists: Entity<Playlists>,
    favorites: Entity<Favorites>,
    search: Entity<Search>,
    player: Entity<Player>,
    queue: Entity<QueueView>,
    /// Over the screen and the player when open; the rest stay as they were.
    lyrics: Entity<LyricsView>,
    mini_player: Entity<MiniPlayer>,
    panes: Entity<ResizableState>,
    /// Holds the session's decoded covers, so signing out frees them.
    covers: Entity<CoverCache>,
    _discord: DiscordStatus,
    _media: SystemMediaControls,
    _media_notices: Task<()>,
    _player_changes: Task<()>,
    _library_events: Subscription,
    _alerts: Task<()>,
    _artists_events: Subscription,
    _playlists_events: Subscription,
    _playlist_choices: Task<()>,
    _favorites_changes: Task<()>,
    _search_events: Subscription,
    _activity_changes: Task<()>,
    _mini_player_changes: Subscription,
    _skip_changes: Task<()>,
    _focus_lost: Subscription,
    _activation: Subscription,
}

impl WorkspaceScreen {
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

/// Puts up each alert, those told before the shell followed them first.
fn follow_alerts(alerts: Alerts, window: &Window, cx: &mut Context<AppShell>) -> Task<()> {
    let Alerts { earlier, later, .. } = alerts;
    if !earlier.is_empty() {
        cx.defer_in(window, move |this, window, cx| {
            for alert in earlier {
                this.tell(alert, window, cx);
            }
        });
    }
    gpui_host::subscribe_in(later, window, cx, |this, alert, window, cx| {
        this.tell(alert, window, cx);
    })
}

impl AppShell {
    pub fn new(
        hiphile: Hiphile,
        updater: Option<Updater>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        window.set_window_title("hiphile");
        let focus = cx.focus_handle();
        let screen = Self::sign_in(&hiphile, window, cx);
        let alerts = follow_alerts(hiphile.alerts(), window, cx);
        // The dictionaries start downloading as the app opens
        hiphile.readings();
        let signing_in =
            gpui_host::observe_in(hiphile.subscribe(), window, cx, Self::follow_signing_in);
        let this = Self {
            _system_appearance_changes: settings::follow_system_appearance(
                hiphile.settings().appearance().clone(),
                window,
            ),
            hiphile,
            focus,
            screen,
            queue_shown: true,
            _alerts: alerts,
            _signing_in: signing_in,
            _switching: Task::ready(()),
            _updater_changes: updater
                .as_ref()
                .map(|updater| gpui_host::notify_on_change(updater.subscribe(), cx)),
            updater,
        };
        this.update_menus(cx);
        this
    }

    fn sign_in(hiphile: &Hiphile, window: &mut Window, cx: &mut Context<Self>) -> Screen {
        Screen::SignIn(cx.new(|cx| SignIn::new(hiphile.clone(), window, cx)))
    }

    /// Shows the workspace of whoever signed in, and the form again once nobody is. The title
    /// bar and the menus show who could be signed in instead.
    fn follow_signing_in(
        &mut self,
        signed_in: HiphileSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match (signed_in.workspace(), &self.screen) {
            (Some(workspace), Screen::Workspace(shown)) if shown.core == *workspace => {}
            (Some(workspace), _) => self.open_workspace(workspace.clone(), window, cx),
            (None, Screen::Workspace(_)) => self.show_signed_out(window, cx),
            (None, Screen::SignIn(_)) => {}
        }
        self.update_menus(cx);
        cx.notify();
    }

    /// Signs in with `profile` in place of whoever is, asking for its password when none is
    /// saved or signing in with it fails: on the form when nobody is signed in, and otherwise
    /// over the workspace, which stays meanwhile.
    fn switch_server(
        &mut self,
        profile: config::Profile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let switching = self.hiphile.switch_to(profile.clone());
        self._switching = cx.spawn_in(window, async move |this, cx| {
            let failure = match switching.await {
                Ok(()) => return,
                Err(SwitchFailed::NoPassword) => None,
                Err(SwitchFailed::SignIn(error)) => Some(SharedString::from(error)),
            };
            this.update_in(cx, |this, window, cx| match &this.screen {
                Screen::SignIn(sign_in) => sign_in.update(cx, |sign_in, cx| {
                    sign_in.show_profile(&profile, failure, window, cx);
                }),
                Screen::Workspace(_) => {
                    account::open_sign_in_dialog(
                        &this.hiphile,
                        Some(&profile),
                        failure,
                        window,
                        cx,
                    );
                }
            })
            .ok();
        });
    }

    fn open_workspace(&mut self, core: Workspace, window: &mut Window, cx: &mut Context<Self>) {
        let session = core.session().clone();
        let (shelves, recommendations) = (core.shelves().clone(), core.recommendations().clone());
        let home = cx.new(|cx| Home::new(session.clone(), shelves, recommendations, window, cx));
        let library = cx.new(|cx| {
            let (sync, albums) = (core.library().clone(), core.albums().clone());
            Library::new(session.clone(), sync, albums, window, cx)
        });
        let artists = cx.new(|cx| {
            let (list, detail) = (core.artists().clone(), core.artist_detail().clone());
            Artists::new(session.clone(), list, detail, window, cx)
        });
        let songs = cx.new(|cx| Songs::new(session.clone(), core.songs().clone(), window, cx));
        let playlists =
            cx.new(|cx| Playlists::new(session.clone(), core.playlists().clone(), window, cx));
        let playlist_choices = follow_playlist_choices(core.playlists(), cx);
        let (favorites, favorites_changes) = Self::favorites(&core, window, cx);
        let search = cx.new(|cx| Search::new(session.clone(), core.search().clone(), window, cx));
        let search_events = cx.subscribe_in(&search, window, Self::on_search_event);
        let activity_changes = follow_library_activity(core.activity().clone(), window, cx);
        let player = new_player(&core, window, cx);
        let queue = cx.new(|cx| QueueView::new(core.player().clone(), window, cx));
        let lyrics = cx.new(|cx| LyricsView::new(player.clone(), core.lyrics().clone(), cx));
        let mini_player =
            cx.new(|cx| MiniPlayer::new(player.clone(), core.lyrics().clone(), window, cx));
        let mini_player_changes = cx.observe(&mini_player, |this, _, cx| {
            this.update_menus(cx);
            cx.notify();
        });
        let discord = core.show_on_discord(presence());
        let media = core.connect_media_controls(media_controls::MediaControls::connect(window));
        let media_notices =
            gpui_host::subscribe_in(media.notices(), window, cx, |_, notice, window, cx| {
                match notice {
                    MediaNotice::Raise => {
                        cx.activate(true);
                        window.activate_window();
                    }
                }
            });
        let player_changes = follow_player(
            core.player(),
            NowPlayingScreens {
                home: home.downgrade(),
                library: library.downgrade(),
                artists: artists.downgrade(),
                songs: songs.downgrade(),
                playlists: playlists.downgrade(),
                favorites: favorites.downgrade(),
            },
            cx,
        );
        let library_events = cx.subscribe(&library, Self::on_library_event);
        let alerts = follow_alerts(core.alerts(), window, cx);
        let artists_events = cx.subscribe(&artists, |this, _, event, cx| match event {
            ArtistsEvent::LeftArtist(artist) => {
                this.remember(Location::Artists(Some(artist.clone())), cx);
            }
        });
        let playlists_events = cx.subscribe_in(&playlists, window, Self::on_playlists_event);
        let skip_changes = follow_skip_instrumentals(self.hiphile.settings().queue(), cx);
        // A focused control that goes away, such as Retry once loading starts,
        // would leave no focus and no working shortcuts.
        let focus_lost = cx.on_focus_lost(window, |this, window, cx| {
            if let Screen::Workspace(workspace) = &this.screen {
                workspace.focus_destination(window, cx);
            }
        });
        let activation = cx.observe_window_activation(window, |this, window, _| {
            if let Screen::Workspace(workspace) = &this.screen
                && window.is_window_active()
            {
                workspace.core.app_activated();
            }
        });
        let workspace = WorkspaceScreen {
            core,
            destination: Destination::Home,
            history: History::new(),
            home,
            library,
            artists,
            songs,
            playlists,
            favorites,
            search,
            player,
            queue,
            lyrics,
            mini_player,
            panes: cx.new(|_| ResizableState::default()),
            covers: CoverCache::new(cx),
            _discord: discord,
            _media: media,
            _media_notices: media_notices,
            _player_changes: player_changes,
            _library_events: library_events,
            _alerts: alerts,
            _artists_events: artists_events,
            _playlists_events: playlists_events,
            _playlist_choices: playlist_choices,
            _favorites_changes: favorites_changes,
            _search_events: search_events,
            _activity_changes: activity_changes,
            _mini_player_changes: mini_player_changes,
            _skip_changes: skip_changes,
            _focus_lost: focus_lost,
            _activation: activation,
        };
        // Each dialog belonged to the workspace this replaces, or signed in to this one
        window.close_all_dialogs(cx);
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
                if let Some(core) = self.core() {
                    core.play(std::slice::from_ref(song), 0);
                }
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

    /// The favorites' screen, and what passes them on to the other screens.
    fn favorites(
        core: &Workspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Entity<Favorites>, Task<()>) {
        let state = core.favorites();
        let view = cx.new(|cx| Favorites::new(core.session().clone(), state.clone(), window, cx));
        (view, follow_favorites(state, cx))
    }

    /// Puts up `alert`, unless the screen showing tells of it itself.
    fn tell(&self, alert: Alert, window: &mut Window, cx: &mut App) {
        if alert.told_on.is_some() && alert.told_on == self.destination() {
            return;
        }
        let kind = match alert.kind {
            AlertKind::Error => NotificationType::Error,
            AlertKind::Info => NotificationType::Info,
        };
        window.push_notification((kind, alert.message), cx);
    }

    fn on_library_event(
        &mut self,
        _: Entity<Library>,
        event: &LibraryEvent,
        cx: &mut Context<Self>,
    ) {
        let LibraryEvent::LeftAlbum(album) = event;
        self.remember(Location::Albums(Some(album.clone())), cx);
    }

    fn destination(&self) -> Option<Destination> {
        match &self.screen {
            Screen::Workspace(workspace) => Some(workspace.destination),
            Screen::SignIn(_) => None,
        }
    }

    fn update_menus(&self, cx: &mut App) {
        let (shuffled, repeat) = self.core().map_or((false, RepeatMode::Off), |core| {
            let shown = core.snapshot();
            (shown.is_shuffled(), shown.repeat())
        });
        let is_lyrics_open = self.is_lyrics_open(cx);
        let state = MenuState {
            queue_shown: self.queue_shown && !is_lyrics_open,
            lyrics_open: is_lyrics_open,
            mini_player_open: self.is_mini_player_open(cx),
            destination: self.destination(),
            shuffled,
            repeat,
            accounts: &self.hiphile.snapshot(),
        };
        crate::set_menus(&state, cx);
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
        workspace.core.showing(destination);
        match destination {
            Destination::Artists => workspace
                .artists
                .update(cx, |artists, cx| artists.activate(cx)),
            Destination::Songs => workspace.songs.update(cx, |songs, cx| songs.activate(cx)),
            Destination::Home
            | Destination::Albums
            | Destination::Playlists
            | Destination::Favorites => {}
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
            Screen::SignIn(_) => (false, false),
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

    /// Closes the workspace, which stops playback and forgets the saved
    /// password, and returns to signing in.
    fn sign_out(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.hiphile.sign_out();
        self.show_signed_out(window, cx);
    }

    fn show_signed_out(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.screen, Screen::SignIn(_)) {
            return;
        }
        // The window keeps its notices, and this one would never hear the end
        window.remove_notification::<ActivityNotice>(cx);
        self.focus.focus(window, cx);
        self.screen = Self::sign_in(&self.hiphile, window, cx);
        self.update_menus(cx);
        cx.notify();
    }

    fn player(&self) -> Option<&Entity<Player>> {
        match &self.screen {
            Screen::Workspace(workspace) => Some(&workspace.player),
            Screen::SignIn(_) => None,
        }
    }

    /// What plays what the player shows.
    fn core(&self) -> Option<playback::Player> {
        match &self.screen {
            Screen::Workspace(workspace) => Some(workspace.core.player().clone()),
            Screen::SignIn(_) => None,
        }
    }

    fn play_songs(&mut self, action: &PlaySongs, _: &mut Window, _: &mut Context<Self>) {
        let Some(core) = self.core() else {
            return;
        };
        match action.start_ix() {
            Some(start_ix) => core.play(action.songs(), start_ix),
            None => core.play_whole(action.songs()),
        };
    }

    fn enqueue_songs(&mut self, action: &EnqueueSongs, _: &mut Window, _: &mut Context<Self>) {
        if let Some(core) = self.core() {
            core.enqueue(action.songs(), action.placement());
        }
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
            Screen::SignIn(_) => false,
        }
    }

    fn is_mini_player_open(&self, cx: &App) -> bool {
        match &self.screen {
            Screen::Workspace(workspace) => workspace.mini_player.read(cx).is_open(),
            Screen::SignIn(_) => false,
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
        let has_song = self.core().is_some_and(|core| core.snapshot().has_song());
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
            .on_action(cx.listener(|this, _: &TogglePlayback, _, _| {
                this.core().inspect(playback::Player::toggle_playback);
            }))
            .on_action(cx.listener(|this, _: &NextSong, _, _| {
                this.core().inspect(playback::Player::next);
            }))
            .on_action(cx.listener(|this, _: &PreviousSong, _, _| {
                this.core().inspect(playback::Player::previous);
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
            Screen::SignIn(_) => None,
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
        workspace: &WorkspaceScreen,
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
                    .children(
                        account::server_menu(&self.hiphile.snapshot())
                            .map(|menu| title_bar_controls().child(menu)),
                    )
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

/// Puts up the activity's notice when it should be up and takes it down when it should not.
fn follow_library_activity(
    activity: LibraryActivity,
    window: &Window,
    cx: &mut Context<AppShell>,
) -> Task<()> {
    let changes = activity.subscribe();
    let mut is_shown = changes.borrow().is_notice_shown();
    let notice = cx.new(|cx| ActivityNotice::new(activity, cx));
    // The library may have got busy before the shell followed it
    if is_shown {
        cx.defer_in(window, {
            let notice = notice.clone();
            move |_, window, cx| {
                window.push_notification(ActivityNotice::notification(&notice), cx);
            }
        });
    }
    gpui_host::observe_in(changes, window, cx, move |_, activity, window, cx| {
        if activity.is_notice_shown() == is_shown {
            return;
        }
        is_shown = activity.is_notice_shown();
        if is_shown {
            window.push_notification(ActivityNotice::notification(&notice), cx);
        } else {
            window.remove_notification::<ActivityNotice>(cx);
        }
    })
}

/// The other screens' copy, by which they dim the songs that are skipped.
fn follow_skip_instrumentals(queue: &QueueSettings, cx: &mut Context<AppShell>) -> Task<()> {
    let changes = queue.subscribe();
    let mut is_on = changes.borrow().is_skip_instrumentals_on();
    actions::set_skip_instrumentals(is_on, cx);
    gpui_host::observe(changes, cx, move |_, queue, cx| {
        if queue.is_skip_instrumentals_on() != is_on {
            is_on = queue.is_skip_instrumentals_on();
            actions::set_skip_instrumentals(is_on, cx);
        }
    })
}

/// The screens that mark the song the player is on.
struct NowPlayingScreens {
    home: WeakEntity<Home>,
    library: WeakEntity<Library>,
    artists: WeakEntity<Artists>,
    songs: WeakEntity<Songs>,
    playlists: WeakEntity<Playlists>,
    favorites: WeakEntity<Favorites>,
}

impl NowPlayingScreens {
    fn set_now_playing(&self, song: Option<&SongId>, playing: bool, cx: &mut App) {
        let song = || song.cloned();
        self.home
            .update(cx, |home, cx| home.set_now_playing(song(), playing, cx))
            .ok();
        self.library
            .update(cx, |library, cx| {
                library.set_now_playing(song(), playing, cx)
            })
            .ok();
        self.artists
            .update(cx, |artists, cx| {
                artists.set_now_playing(song(), playing, cx)
            })
            .ok();
        self.songs
            .update(cx, |songs, cx| songs.set_now_playing(song(), playing, cx))
            .ok();
        self.playlists
            .update(cx, |playlists, cx| {
                playlists.set_now_playing(song(), playing, cx)
            })
            .ok();
        self.favorites
            .update(cx, |favorites, cx| {
                favorites.set_now_playing(song(), playing, cx)
            })
            .ok();
    }
}

/// Every screen marks the song the player is on; the shell's menus show the shuffle and the
/// repeat, and it draws again for which commands are available.
fn follow_player(
    core: &playback::Player,
    screens: NowPlayingScreens,
    cx: &mut Context<AppShell>,
) -> Task<()> {
    let now_playing = |shown: &PlayerSnapshot| {
        let song = shown.current_song().map(|song| song.id().clone());
        (song, shown.is_playing())
    };
    let order = |shown: &PlayerSnapshot| (shown.is_shuffled(), shown.repeat());
    let commands = |shown: &PlayerSnapshot| (shown.has_song(), shown.has_queued_songs());
    let changes = core.subscribe();
    let (mut playing, mut ordered, mut commanded) = {
        let shown = changes.borrow();
        (now_playing(&shown), order(&shown), commands(&shown))
    };
    gpui_host::observe(changes, cx, move |this, shown, cx| {
        if now_playing(&shown) != playing {
            playing = now_playing(&shown);
            // With no song the lyrics have nothing to show. Focus follows once they are
            // gone, as the view holds it.
            if playing.0.is_none() {
                this.close_lyrics(cx);
            }
            screens.set_now_playing(playing.0.as_ref(), playing.1, cx);
            cx.notify();
        }
        if order(&shown) != ordered {
            ordered = order(&shown);
            this.update_menus(cx);
            cx.notify();
        }
        if commands(&shown) != commanded {
            commanded = commands(&shown);
            cx.notify();
        }
    })
}

/// The choices of the last session go at once.
fn follow_playlist_choices(playlists: &PlaylistsState, cx: &mut Context<AppShell>) -> Task<()> {
    let changes = playlists.subscribe();
    let mut choices = changes.borrow().choices().clone();
    actions::set_playlist_choices(choices.clone(), cx);
    gpui_host::observe(changes, cx, move |_, playlists, cx| {
        if *playlists.choices() != choices {
            choices = playlists.choices().clone();
            actions::set_playlist_choices(choices.clone(), cx);
        }
    })
}

/// Only a change of what is a favorite reaches the other screens, which all draw again.
fn follow_favorites<V: 'static>(favorites: &FavoritesState, cx: &mut Context<V>) -> Task<()> {
    let changes = favorites.subscribe();
    let mut known = changes.borrow().known().cloned();
    // Those of whoever was signed in before go
    actions::set_known_favorites(known.as_deref().cloned(), cx);
    gpui_host::observe(changes, cx, move |_, favorites, cx| {
        if favorites.known() != known.as_ref() {
            known = favorites.known().cloned();
            actions::set_known_favorites(known.as_deref().cloned(), cx);
        }
    })
}

/// Tests play made-up songs, which must not reach the user's status.
fn presence() -> Presence {
    if cfg!(test) {
        Presence::disconnected()
    } else {
        Presence::spawn()
    }
}

/// The title bar moves the window, and on Windows a press that goes on to the title bar starts
/// that move, which takes the release and so the click.
fn title_bar_controls() -> Div {
    h_flex()
        .flex_none()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

fn new_player(core: &Workspace, window: &mut Window, cx: &mut Context<AppShell>) -> Entity<Player> {
    let shell = cx.weak_entity();
    cx.new(|cx| {
        let mut player = Player::new(core.player().clone(), core.session().clone(), window, cx);
        player.set_panels(move |window, cx| {
            shell
                .upgrade()
                .map_or_else(Panels::default, |shell| shell.read(cx).panels(window, cx))
        });
        player
    })
}

impl Render for AppShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let queue_fits = self.queue_fits(window, cx);
        let queue_visible = self.queue_shown && queue_fits;
        let content: AnyElement = match &self.screen {
            Screen::SignIn(view) => view.clone().into_any_element(),
            Screen::Workspace(workspace) => {
                self.render_workspace(workspace, queue_visible, window, cx)
            }
        };
        let covers = match &self.screen {
            Screen::Workspace(workspace) => Some(workspace.covers.clone()),
            Screen::SignIn(_) => None,
        };
        let signed_in = matches!(self.screen, Screen::Workspace(_));
        let signing_in = self.hiphile.snapshot().is_signing_in();
        let has_queue = self
            .core()
            .is_some_and(|core| core.snapshot().has_queued_songs());
        let chooser = match &self.screen {
            Screen::Workspace(workspace) => workspace.playlists.read(cx).render_chooser(window),
            Screen::SignIn(_) => None,
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
            .when(!signing_in, |this| {
                this.on_action(cx.listener(|this, action: &SwitchServer, window, cx| {
                    this.switch_server(action.profile().clone(), window, cx);
                }))
            })
            .when(signed_in && !signing_in, |this| {
                this.on_action(cx.listener(|this, _: &AddServer, window, cx| {
                    account::open_sign_in_dialog(&this.hiphile, None, None, window, cx);
                }))
            })
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
                .on_action(cx.listener(|this, action: &ToggleFavoriteOf, _, _| {
                    if let Screen::Workspace(workspace) = &this.screen {
                        let favorite = action.favorite().clone();
                        workspace.core.favorites().toggle(favorite);
                    }
                }))
                .on_action(cx.listener(|this, action: &ShowAlbum, window, cx| {
                    this.show_album(action.album(), window, cx);
                }))
                .on_action(cx.listener(|this, action: &ShowArtist, window, cx| {
                    this.show_artist(action.artist(), window, cx);
                }))
                .on_action(cx.listener(|this, _: &Refresh, _, _| {
                    if let Screen::Workspace(workspace) = &this.screen {
                        workspace.core.refresh(workspace.destination);
                    }
                }))
                .on_action(cx.listener(|this, action: &AddToPlaylist, _, _| {
                    if let Screen::Workspace(workspace) = &this.screen {
                        workspace
                            .core
                            .playlists()
                            .add_songs(action.playlist(), action.songs());
                    }
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
                .on_action(cx.listener(|this, _: &VolumeUp, _, _| {
                    this.core().inspect(playback::Player::volume_up);
                }))
                .on_action(cx.listener(|this, _: &VolumeDown, _, _| {
                    this.core().inspect(playback::Player::volume_down);
                }))
                .on_action(cx.listener(|this, _: &ToggleMute, _, _| {
                    this.core().inspect(playback::Player::toggle_mute);
                }))
                .on_action(cx.listener(|this, _: &ToggleShuffle, _, _| {
                    this.core().inspect(playback::Player::toggle_shuffle);
                }))
                .on_action(cx.listener(|this, _: &CycleRepeat, _, _| {
                    this.core().inspect(playback::Player::cycle_repeat);
                }))
                .on_action(cx.listener(|this, action: &SetRepeat, _, _| {
                    if let Some(core) = this.core() {
                        core.set_repeat(action.repeat());
                    }
                }))
            })
            .when(queue_fits, |this| {
                this.on_action(cx.listener(|this, _: &ToggleQueue, window, cx| {
                    this.toggle_queue(window, cx);
                }))
            })
            .when(has_queue, |this| {
                this.on_action(cx.listener(|this, _: &SaveQueue, window, cx| {
                    let Some(songs) = this.core().map(|core| core.snapshot().queued_songs()) else {
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
