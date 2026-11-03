//! App shell: initializes GPUI Kit, installs shared services, and composes
//! feature crates into the window. Feature logic lives in the feature crates.

mod app_shell;
mod assets;

use gpui_kit::component::TitleBar;
use gpui_kit::{
    App, AppContext as _, KeyBinding, Menu, MenuItem, WindowBounds, WindowOptions, actions, px,
    size,
};
use player::{
    NextSong, PreviousSong, SeekBackward, SeekForward, ToggleMute, TogglePlayback, VolumeDown,
    VolumeUp,
};

use app_shell::{AppShell, WORKSPACE_CONTEXT};

actions!(hiphile, [Quit, SignOut]);

fn main() {
    gpui_kit::application()
        .with_assets(assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);

            let user_agent = concat!("hiphile/", env!("CARGO_PKG_VERSION"));
            let net = net::Client::new(user_agent).expect("failed to start the network runtime");
            session::init(net.clone(), cx);

            library::init(cx);
            player::init(WORKSPACE_CONTEXT, cx);
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
    let quit = if cx.platform_info().is_mac() {
        "cmd-q"
    } else {
        "ctrl-q"
    };
    cx.bind_keys([
        KeyBinding::new(quit, Quit, None),
        // Space toggles playback from the lists, never from a text field.
        KeyBinding::new("space", TogglePlayback, Some(library::ALBUMS_CONTEXT)),
        KeyBinding::new("space", TogglePlayback, Some(library::SONGS_CONTEXT)),
    ]);
    cx.set_menus([
        Menu::new("hiphile").items([
            MenuItem::action("Sign Out", SignOut),
            MenuItem::separator(),
            MenuItem::action("Quit hiphile", Quit),
        ]),
        Menu::new("Library").items([MenuItem::action("Refresh Library", library::Refresh)]),
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
