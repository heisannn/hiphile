//! The settings window. The settings themselves are in `hiphile_core::settings`, kept by the app's
//! core. The appearance only concerns the theme, which this crate keeps on it.

mod appearance_settings;
mod audio_options;
mod cache_limit_field;
mod dsd_section;
mod imgbb_key_field;
mod number_field;
mod r8brain_number_field;
mod r8brain_section;
mod settings_view;
mod soxr_section;

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::TitleBar;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, KeyBinding, Subscription, TitlebarOptions,
    Window, WindowBounds, WindowOptions, actions, px, size,
};
use hiphile_core::Hiphile;
use hiphile_core::settings::AppearanceSettings;

use settings_view::SettingsView;
use updater::Updater;

const SETTINGS_CONTEXT: &str = "Settings";

actions!(settings, [OpenSettings, CloseWindow]);

/// Puts the theme on the chosen appearance, and shows the settings of `hiphile` on
/// [`OpenSettings`]. `updater` is `None` in a build that does not update itself. Call before
/// building the menu bar.
pub fn init(hiphile: Hiphile, updater: Option<Updater>, cx: &mut App) {
    number_field::init(cx);
    let settings = hiphile.settings();
    appearance_settings::apply(settings.appearance(), cx);
    settings.audio().list_devices();
    // It may have been closed since
    let window = Rc::new(Cell::new(None));
    // Deferred because an action reaches this handler while its window is being
    // updated. That window cannot be updated again until then, which `open`
    // would take to mean that the settings window was closed.
    cx.on_action(move |_: &OpenSettings, cx| {
        let (hiphile, updater, window) = (hiphile.clone(), updater.clone(), window.clone());
        cx.defer(move |cx| open(&hiphile, updater, &window, cx));
    });
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

/// Keeps the theme on the system's appearance for as long as the subscription
/// lives, if that is being followed. Only a window reports a change, so each
/// window that can be the last one open holds one.
pub fn follow_system_appearance(appearance: AppearanceSettings, window: &Window) -> Subscription {
    window.observe_window_appearance(move |window, cx| {
        appearance_settings::on_system_appearance_changed(&appearance, window, cx);
    })
}

fn open(
    hiphile: &Hiphile,
    updater: Option<Updater>,
    opened: &Cell<Option<AnyWindowHandle>>,
    cx: &mut App,
) {
    if let Some(window) = opened.get()
        && window
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }
    // Window geometry is a platform boundary, so it is given in pixels
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
    let settings = hiphile.settings().clone();
    let readings = hiphile.readings().clone();
    let song_cache_usage = hiphile.song_cache_usage();
    let window = gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| SettingsView::new(settings, readings, song_cache_usage, updater, window, cx))
    });
    if let Ok((window, _)) = window {
        opened.set(Some(window));
    }
}
