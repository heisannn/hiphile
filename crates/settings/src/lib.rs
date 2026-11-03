//! Feature: the settings window.
//!
//! [`AudioSettings`] holds the chosen output and saves it; the app shell
//! hands it to the player and passes on [`OutputChanged`].

mod audio_settings;
mod settings_view;

use gpui_kit::component::TitleBar;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Entity, Global, KeyBinding, TitlebarOptions,
    WindowBounds, WindowOptions, actions, px, size,
};

pub use audio_settings::{AudioSettings, OutputChanged};

use settings_view::SettingsView;

/// Key context of the settings window.
const SETTINGS_CONTEXT: &str = "Settings";

actions!(settings, [OpenSettings, CloseWindow]);

struct GlobalSettings {
    audio: Entity<AudioSettings>,
    /// The settings window, if one was opened. It may have been closed since.
    window: Option<AnyWindowHandle>,
}

impl Global for GlobalSettings {}

/// Reads the settings and registers commands. Call before building the menu
/// bar.
pub fn init(cx: &mut App) {
    let audio = cx.new(AudioSettings::new);
    cx.set_global(GlobalSettings {
        audio,
        window: None,
    });
    // Deferred because an action reaches this handler while its window is being
    // updated. That window cannot be updated again until then, which `open`
    // would take to mean that the settings window was closed.
    cx.on_action(|_: &OpenSettings, cx| cx.defer(open));
    let (open, close) = if cx.platform_info().is_mac() {
        ("cmd-,", "cmd-w")
    } else {
        ("ctrl-,", "ctrl-w")
    };
    cx.bind_keys([
        KeyBinding::new(open, OpenSettings, None),
        KeyBinding::new(close, CloseWindow, Some(SETTINGS_CONTEXT)),
    ]);
}

/// The chosen output, shared by the settings window and the player.
pub fn audio_settings(cx: &App) -> Entity<AudioSettings> {
    cx.global::<GlobalSettings>().audio.clone()
}

/// Brings the settings window forward, opening it if it is not open.
fn open(cx: &mut App) {
    if let Some(window) = cx.global::<GlobalSettings>().window
        && window
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }
    // Window geometry is a platform boundary, so it is given in pixels.
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(800.), px(520.)),
            cx,
        ))),
        window_min_size: Some(size(px(560.), px(360.))),
        titlebar: Some(TitlebarOptions {
            title: Some("Settings".into()),
            ..TitleBar::title_bar_options()
        }),
        ..TitleBar::window_options()
    };
    let audio = audio_settings(cx);
    let opened = gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| SettingsView::new(audio, window, cx))
    });
    if let Ok((window, _)) = opened {
        cx.global_mut::<GlobalSettings>().window = Some(window);
    }
}
