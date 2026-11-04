//! App shell: initializes GPUI Kit, installs shared services, and composes
//! feature crates into the window. Feature logic lives in the feature crates.

mod app_shell;
mod assets;
mod navigation;

use gpui_kit::component::TitleBar;
use gpui_kit::{
    App, AppContext as _, KeyBinding, Menu, MenuItem, WindowBounds, WindowOptions, actions, px,
    size,
};
use player::{
    NextSong, PreviousSong, SaveQueue, SeekBackward, SeekForward, ToggleMute, TogglePlayback,
    VolumeDown, VolumeUp,
};

use app_shell::{AppShell, Destination, WORKSPACE_CONTEXT};

actions!(
    hiphile,
    [
        Quit,
        SignOut,
        ToggleQueue,
        ShowHome,
        ShowAlbums,
        ShowArtists,
        ShowSongs,
        ShowPlaylists,
        GoBack,
        GoForward
    ]
);

fn main() {
    gpui_kit::application()
        .with_assets(assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);

            let user_agent = concat!("hiphile/", env!("CARGO_PKG_VERSION"));
            let net = net::Client::new(user_agent).expect("failed to start the network runtime");
            session::init(net.clone(), cx);

            ui::list_navigation::bind_control_keys(cx);
            home::init(cx);
            library::init(cx);
            artists::init(cx);
            songs::init(cx);
            player::init(WORKSPACE_CONTEXT, cx);
            playlists::init(cx);
            settings::init(cx);
            install_app_commands(cx);

            // Window geometry is a platform boundary, so it is given in pixels.
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1200.), px(760.)), cx)),
                window_min_size: Some(size(px(900.), px(560.))),
                app_id: Some("dev.heisannn.hiphile".into()),
                ..TitleBar::window_options()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| AppShell::new(net, window, cx))
            })
            .expect("failed to open the main window");
            cx.activate(true);
        });
}

fn install_app_commands(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
    // Bind keys before `set_menus`: menus read the keymap when they are built.
    let [
        quit,
        toggle_queue,
        show_home,
        show_artists,
        show_albums,
        show_songs,
        show_playlists,
        refresh,
        new_playlist,
        go_back,
        go_forward,
    ] = if cx.platform_info().is_mac() {
        [
            "cmd-q",
            "cmd-alt-u",
            "cmd-1",
            "cmd-2",
            "cmd-3",
            "cmd-4",
            "cmd-5",
            "cmd-r",
            "cmd-n",
            "cmd-[",
            "cmd-]",
        ]
    } else {
        [
            "ctrl-q",
            "ctrl-alt-u",
            "ctrl-1",
            "ctrl-2",
            "ctrl-3",
            "ctrl-4",
            "ctrl-5",
            "ctrl-r",
            "ctrl-n",
            "alt-left",
            "alt-right",
        ]
    };
    cx.bind_keys([
        KeyBinding::new(quit, Quit, None),
        KeyBinding::new(toggle_queue, ToggleQueue, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new(show_home, ShowHome, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new(show_artists, ShowArtists, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new(show_albums, ShowAlbums, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new(show_songs, ShowSongs, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new(show_playlists, ShowPlaylists, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new(go_back, GoBack, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new(go_forward, GoForward, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new(refresh, library::Refresh, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new(
            new_playlist,
            actions::NewPlaylist::empty(),
            Some(WORKSPACE_CONTEXT),
        ),
        // Space toggles playback from the lists, never from a text field.
        KeyBinding::new("space", TogglePlayback, Some(home::HOME_CONTEXT)),
        KeyBinding::new("space", TogglePlayback, Some(library::ALBUMS_CONTEXT)),
        KeyBinding::new("space", TogglePlayback, Some(library::SONGS_CONTEXT)),
        KeyBinding::new("space", TogglePlayback, Some(artists::ARTISTS_CONTEXT)),
        KeyBinding::new("space", TogglePlayback, Some(artists::DETAIL_CONTEXT)),
        KeyBinding::new("space", TogglePlayback, Some(songs::SONGS_CONTEXT)),
        KeyBinding::new("space", TogglePlayback, Some(playlists::PLAYLISTS_CONTEXT)),
        KeyBinding::new("space", TogglePlayback, Some(playlists::SONGS_CONTEXT)),
    ]);
}

/// Builds the menu bar; `queue_shown` checks View > Queue, and `destination`
/// the screen showing, if any. The app shell calls it, once the keys above are
/// bound (menus read the keymap when they are built), and again when that
/// changes.
pub(crate) fn set_menus(queue_shown: bool, destination: Option<Destination>, cx: &mut App) {
    cx.set_menus([
        Menu::new("hiphile").items([
            MenuItem::action("Settings…", settings::OpenSettings),
            MenuItem::separator(),
            MenuItem::action("Sign Out", SignOut),
            MenuItem::separator(),
            MenuItem::action("Quit hiphile", Quit),
        ]),
        Menu::new("Library").items([
            MenuItem::action("Refresh Library", library::Refresh),
            MenuItem::separator(),
            MenuItem::action("New Playlist…", actions::NewPlaylist::empty()),
            MenuItem::action("Save Queue as Playlist…", SaveQueue),
        ]),
        Menu::new("View").items([
            MenuItem::action("Back", GoBack),
            MenuItem::action("Forward", GoForward),
            MenuItem::separator(),
            MenuItem::action("Home", ShowHome).checked(destination == Some(Destination::Home)),
            MenuItem::action("Artists", ShowArtists)
                .checked(destination == Some(Destination::Artists)),
            MenuItem::action("Albums", ShowAlbums)
                .checked(destination == Some(Destination::Albums)),
            MenuItem::action("Songs", ShowSongs).checked(destination == Some(Destination::Songs)),
            MenuItem::action("Playlists", ShowPlaylists)
                .checked(destination == Some(Destination::Playlists)),
            MenuItem::separator(),
            MenuItem::action("Queue", ToggleQueue).checked(queue_shown),
        ]),
        Menu::new("Controls").items([
            MenuItem::action("Play/Pause", TogglePlayback),
            MenuItem::action("Next", NextSong),
            MenuItem::action("Previous", PreviousSong),
            MenuItem::separator(),
            MenuItem::action("Seek Forward", SeekForward),
            MenuItem::action("Seek Backward", SeekBackward),
            MenuItem::separator(),
            MenuItem::action("Volume Up", VolumeUp),
            MenuItem::action("Volume Down", VolumeDown),
            MenuItem::action("Mute", ToggleMute),
        ]),
    ]);
}
