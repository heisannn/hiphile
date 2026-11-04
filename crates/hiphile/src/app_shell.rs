use account::{SignIn, SignInEvent, Start};
use actions::{
    AddToPlaylist, ChoosePlaylist, EnqueueSongs, NewPlaylist, PlaySongs, ShowAlbum, ShowArtist,
};
use artists::Artists;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, StyledExt as _, TitleBar,
    WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    notification::NotificationType,
    resizable::{ResizableState, h_resizable, resizable_panel},
    tab::{Tab, TabBar},
    v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable as _,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Render, SharedString,
    Styled as _, Subscription, Window, div, prelude::FluentBuilder as _,
};
use home::Home;
use library::{Library, LibraryEvent, Refresh};
use player::{
    NextSong, Player, PlayerEvent, PreviousSong, QueueView, SaveQueue, SeekBackward, SeekForward,
    ToggleMute, TogglePlayback, VolumeDown, VolumeUp,
};
use playlists::{Playlists, PlaylistsEvent};
use session::Session;
use songs::Songs;
use subsonic::{AlbumId, ArtistId};
use ui::CoverCache;

use crate::{ShowAlbums, ShowArtists, ShowHome, ShowPlaylists, ShowSongs, SignOut, ToggleQueue};

/// The queue's width when first shown, in rem.
const QUEUE_WIDTH: f32 = 20.;

/// Key context of the whole window; player shortcuts are bound to it.
pub const WORKSPACE_CONTEXT: &str = "Workspace";

/// Root view of the main window. Decides between signing in and the signed-in
/// workspace, and routes commands between features; it owns no feature state.
pub struct AppShell {
    net: net::Client,
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
    _song_cache_changes: Subscription,
    _cover_cache_changes: Subscription,
    _system_appearance_changes: Subscription,
}

enum Screen {
    SignIn {
        view: Entity<SignIn>,
        _subscription: Subscription,
    },
    Workspace(Box<Workspace>),
}

/// The screen beside the queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Destination {
    Home,
    Artists,
    Albums,
    Songs,
    Playlists,
}

impl Destination {
    /// In the order the title bar shows them.
    const ALL: [Self; 5] = [
        Self::Home,
        Self::Artists,
        Self::Albums,
        Self::Songs,
        Self::Playlists,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Artists => "Artists",
            Self::Albums => "Albums",
            Self::Songs => "Songs",
            Self::Playlists => "Playlists",
        }
    }
}

/// Everything that exists only while signed in.
struct Workspace {
    session: Session,
    profile: config::Profile,
    destination: Destination,
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
    player: Entity<Player>,
    queue: Entity<QueueView>,
    /// The split between the library and the queue.
    panes: Entity<ResizableState>,
    /// Holds the session's decoded covers, so signing out frees them.
    covers: Entity<CoverCache>,
    _player_events: Subscription,
    _library_events: Subscription,
    _playlists_events: Subscription,
    _output_changes: Subscription,
    _cover_key_changes: Subscription,
    _focus_lost: Subscription,
}

impl Workspace {
    /// Gives focus to the screen showing, whose shortcuts then work.
    fn focus_destination(&self, window: &mut Window, cx: &mut App) {
        let focus = match self.destination {
            Destination::Home => self.home.focus_handle(cx),
            Destination::Albums => self.library.focus_handle(cx),
            Destination::Artists => self.artists.focus_handle(cx),
            Destination::Songs => self.songs.focus_handle(cx),
            Destination::Playlists => self.playlists.focus_handle(cx),
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

impl AppShell {
    pub fn new(net: net::Client, window: &mut Window, cx: &mut Context<Self>) -> Self {
        window.set_window_title("hiphile");
        let focus = cx.focus_handle();
        let keychain = config::Keychain::system().ok();
        let screen = Self::sign_in(&net, keychain.clone(), Start::Resume, window, cx);
        let queue_shown = true;
        crate::set_menus(queue_shown, None, cx);
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
        Self {
            net,
            keychain,
            focus,
            screen,
            queue_shown,
            _song_cache_changes: song_cache_changes,
            _cover_cache_changes: cover_cache_changes,
            _system_appearance_changes: settings::follow_system_appearance(window),
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
        let audio_settings = settings::audio_settings(cx);
        let output = audio_settings.read(cx).output().clone();
        let discord_settings = settings::discord_settings(cx);
        let cover_key = discord_settings.read(cx).imgbb_key().map(str::to_owned);
        let player = cx.new(|cx| {
            let mut player = Player::new(session.clone(), output, window, cx);
            player.set_cover_key(cover_key.as_deref());
            player
        });
        let queue = cx.new(|cx| QueueView::new(player.clone(), window, cx));
        let covers = CoverCache::new(cx);
        // Every screen marks the song the player is on; the shell also
        // re-renders, which updates which commands are available.
        let player_events = cx.subscribe(&player, {
            let home = home.downgrade();
            let library = library.downgrade();
            let artists = artists.downgrade();
            let songs = songs.downgrade();
            let playlists = playlists.downgrade();
            move |_, _, event, cx| {
                match event {
                    PlayerEvent::NowPlaying { song, playing } => {
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
                    PlayerEvent::QueueChanged => {}
                }
                cx.notify();
            }
        });
        let library_events = cx.subscribe_in(&library, window, {
            let home = home.downgrade();
            let artists = artists.downgrade();
            let songs = songs.downgrade();
            move |this, _, event, window, cx| match event {
                LibraryEvent::AlbumsSynced => {
                    home.update(cx, |home, cx| home.library_synced(cx)).ok();
                    artists.update(cx, |artists, cx| artists.refresh(cx)).ok();
                }
                LibraryEvent::SongsSynced => {
                    songs.update(cx, |songs, cx| songs.songs_synced(cx)).ok();
                    artists
                        .update(cx, |artists, cx| artists.songs_synced(cx))
                        .ok();
                }
                LibraryEvent::SyncFailed(error) => {
                    // The albums' screen says so itself.
                    if let Screen::Workspace(workspace) = &this.screen
                        && workspace.destination != Destination::Albums
                    {
                        let message = format!("Couldn't refresh the library. {error}");
                        window.push_notification((NotificationType::Error, message), cx);
                    }
                }
            }
        });
        let playlists_events =
            cx.subscribe_in(
                &playlists,
                window,
                |this, _, event, window, cx| match event {
                    PlaylistsEvent::CreatedEmpty => {
                        this.show_destination(Destination::Playlists, window, cx);
                    }
                    PlaylistsEvent::ChooserChanged => cx.notify(),
                },
            );
        let output_changes = cx.subscribe(&audio_settings, {
            let player = player.downgrade();
            move |_, audio_settings, _: &settings::OutputChanged, cx| {
                let output = audio_settings.read(cx).output().clone();
                player
                    .update(cx, |player, cx| player.set_output(output, cx))
                    .ok();
            }
        });
        let cover_key_changes = cx.subscribe(&discord_settings, {
            let player = player.downgrade();
            move |_, discord_settings, _: &settings::ImgbbKeyChanged, cx| {
                let key = discord_settings.read(cx).imgbb_key().map(str::to_owned);
                player
                    .update(cx, |player, _| player.set_cover_key(key.as_deref()))
                    .ok();
            }
        });
        // A focused control that goes away, such as Retry once loading starts,
        // would leave no focus and no working shortcuts.
        let focus_lost = cx.on_focus_lost(window, |this, window, cx| {
            if let Screen::Workspace(workspace) = &this.screen {
                workspace.focus_destination(window, cx);
            }
        });
        let workspace = Workspace {
            session,
            profile,
            destination: Destination::Home,
            home,
            library,
            artists,
            songs,
            playlists,
            player,
            queue,
            panes: cx.new(|_| ResizableState::default()),
            covers,
            _player_events: player_events,
            _library_events: library_events,
            _playlists_events: playlists_events,
            _output_changes: output_changes,
            _cover_key_changes: cover_key_changes,
            _focus_lost: focus_lost,
        };
        workspace.focus_destination(window, cx);
        self.screen = Screen::Workspace(Box::new(workspace));
        self.update_menus(cx);
        cx.notify();
    }

    fn destination(&self) -> Option<Destination> {
        match &self.screen {
            Screen::Workspace(workspace) => Some(workspace.destination),
            Screen::SignIn { .. } => None,
        }
    }

    fn update_menus(&self, cx: &mut App) {
        crate::set_menus(self.queue_shown, self.destination(), cx);
    }

    fn show_destination(
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
            // Another device may have changed them meanwhile.
            Destination::Playlists => workspace
                .playlists
                .update(cx, |playlists, cx| playlists.refresh(window, cx)),
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

    /// Shows `artist` on the artists' screen, whichever screen is showing.
    fn show_artist(&mut self, artist: &ArtistId, window: &mut Window, cx: &mut Context<Self>) {
        self.show_destination(Destination::Artists, window, cx);
        if let Screen::Workspace(workspace) = &self.screen {
            workspace.artists.update(cx, |artists, cx| {
                artists.reveal_artist(artist.clone(), window, cx)
            });
        }
    }

    /// Shows `album` on the albums' screen, whichever screen is showing.
    fn show_album(&mut self, album: &AlbumId, window: &mut Window, cx: &mut Context<Self>) {
        self.show_destination(Destination::Albums, window, cx);
        if let Screen::Workspace(workspace) = &self.screen {
            workspace.library.update(cx, |library, cx| {
                library.reveal_album(album.clone(), window, cx)
            });
        }
    }

    /// Fetches what the screen showing is about again, for the screens that
    /// do not run it themselves.
    fn refresh(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Screen::Workspace(workspace) = &self.screen else {
            return;
        };
        if workspace.destination == Destination::Playlists {
            workspace
                .playlists
                .update(cx, |playlists, cx| playlists.refresh(window, cx));
        } else {
            workspace
                .library
                .update(cx, |library, cx| library.refresh_library(cx));
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

    fn toggle_queue(&mut self, cx: &mut Context<Self>) {
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

    /// The screen showing and, when shown, the queue to its right.
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

    fn render_title_bar(
        &self,
        queue_visible: bool,
        queue_fits: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let server = match &self.screen {
            Screen::Workspace(workspace) => Some(&workspace.session),
            Screen::SignIn { .. } => None,
        };
        let destination_ix = self
            .destination()
            .and_then(|current| Destination::ALL.iter().position(|d| *d == current));
        TitleBar::new()
            .child(
                h_flex()
                    .size_full()
                    .px_1()
                    .pr_3()
                    .gap_2()
                    .when_some(server, |this, session| {
                        let software = match (
                            session.server().server_type(),
                            session.server().server_version(),
                        ) {
                            (Some(name), Some(version)) => Some(format!("{name} {version}")),
                            (Some(name), None) => Some(name.to_owned()),
                            _ => None,
                        };
                        this.child(
                            div()
                                .text_sm()
                                .font_medium()
                                .child(SharedString::from(session.host().to_owned())),
                        )
                        .when_some(software, |this, software| {
                            this.child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(software),
                            )
                        })
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
                        .child(div().flex_1())
                        .child(
                            Button::new("toggle-queue")
                                .ghost()
                                .small()
                                .icon(IconName::ListMusic)
                                .selected(queue_visible)
                                .disabled(!queue_fits)
                                .accessibility_label("Queue")
                                .map(|this| {
                                    if queue_fits {
                                        this.tooltip_with_action("Queue", &ToggleQueue, None)
                                    } else {
                                        this.tooltip("Widen the window to show the queue.")
                                    }
                                })
                                .on_click(cx.listener(|this, _, _, cx| this.toggle_queue(cx))),
                        )
                    }),
            )
            .into_any_element()
    }
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
        let has_song = self
            .player()
            .is_some_and(|player| player.read(cx).has_song());
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
                .on_action(cx.listener(|this, _: &ShowSongs, window, cx| {
                    this.show_destination(Destination::Songs, window, cx);
                }))
                .on_action(cx.listener(|this, _: &ShowPlaylists, window, cx| {
                    this.show_destination(Destination::Playlists, window, cx);
                }))
                .on_action(cx.listener(|this, action: &ShowAlbum, window, cx| {
                    this.show_album(action.album(), window, cx);
                }))
                .on_action(cx.listener(|this, action: &ShowArtist, window, cx| {
                    this.show_artist(action.artist(), window, cx);
                }))
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
                .on_action(cx.listener(|this, action: &PlaySongs, window, cx| {
                    this.update_player(window, cx, |player, _, cx| {
                        player.play(action.songs(), action.start_ix(), cx)
                    });
                }))
                .on_action(cx.listener(|this, action: &EnqueueSongs, window, cx| {
                    let placement = match action.placement() {
                        actions::Placement::Next => player::Placement::Next,
                        actions::Placement::Last => player::Placement::Last,
                    };
                    this.update_player(window, cx, |player, _, cx| {
                        player.enqueue(action.songs(), placement, cx)
                    });
                }))
                .on_action(cx.listener(|this, _: &VolumeUp, window, cx| {
                    this.update_player(window, cx, Player::volume_up);
                }))
                .on_action(cx.listener(|this, _: &VolumeDown, window, cx| {
                    this.update_player(window, cx, Player::volume_down);
                }))
                .on_action(cx.listener(|this, _: &ToggleMute, window, cx| {
                    this.update_player(window, cx, |player, _, cx| player.toggle_mute(cx));
                }))
            })
            .when(queue_fits, |this| {
                this.on_action(cx.listener(|this, _: &ToggleQueue, _, cx| this.toggle_queue(cx)))
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
            .when(has_song, |this| {
                this.on_action(cx.listener(|this, _: &TogglePlayback, window, cx| {
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
            .child(self.render_title_bar(queue_visible, queue_fits, cx))
            .child(div().flex_1().min_h_0().child(content))
            .when_some(self.player().cloned(), |this, player| this.child(player))
            .children(chooser)
    }
}
