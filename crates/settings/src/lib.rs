//! The settings window. The settings themselves are in `hiphile_core::settings`; this crate keeps
//! them for the app, and the app shell hands them to those who use them. The appearance only
//! concerns the theme, which this crate keeps on it.

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

use gpui_kit::component::TitleBar;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Global, KeyBinding, Subscription,
    TitlebarOptions, Window, WindowBounds, WindowOptions, actions, px, size,
};
use hiphile_core::settings::{
    AppearanceSettings, AudioSettings, CacheKind, CacheLimit, DiscordSettings, LyricsSettings,
    QueueSettings,
};

use settings_view::SettingsView;
use updater::Updater;

const SETTINGS_CONTEXT: &str = "Settings";

actions!(settings, [OpenSettings, CloseWindow]);

struct GlobalSettings {
    appearance: AppearanceSettings,
    audio: AudioSettings,
    song_cache: CacheLimit,
    cover_cache: CacheLimit,
    discord: DiscordSettings,
    lyrics: LyricsSettings,
    queue: QueueSettings,
    /// `None` in a build that does not update itself.
    updater: Option<Updater>,
    /// It may have been closed since.
    window: Option<AnyWindowHandle>,
    _quit: Subscription,
}

impl Global for GlobalSettings {}

/// Call before building the menu bar.
pub fn init(updater: Option<Updater>, cx: &mut App) {
    number_field::init(cx);
    let executor = gpui_host::executor(cx);
    let appearance = AppearanceSettings::new(executor.clone(), config::Store::user());
    appearance_settings::apply(&appearance, cx);
    let audio = AudioSettings::new(executor.clone(), config::Store::user());
    audio.list_devices();
    let song_cache = CacheLimit::new(CacheKind::Songs, executor.clone(), config::Store::user());
    let cover_cache = CacheLimit::new(CacheKind::Covers, executor.clone(), config::Store::user());
    let discord = DiscordSettings::new(executor.clone(), config::Store::user());
    let lyrics = LyricsSettings::new(executor.clone(), config::Store::user());
    let queue = QueueSettings::new(executor, config::Store::user());
    // Nothing would write what is still settling or waiting once the app has quit
    let quit = cx.on_app_quit({
        let (song_cache, cover_cache, discord) =
            (song_cache.clone(), cover_cache.clone(), discord.clone());
        move |_| {
            song_cache.save_before_quitting();
            cover_cache.save_before_quitting();
            discord.save_before_quitting();
            async {}
        }
    });
    cx.set_global(GlobalSettings {
        appearance,
        audio,
        song_cache,
        cover_cache,
        discord,
        lyrics,
        queue,
        updater,
        window: None,
        _quit: quit,
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

pub fn audio_settings(cx: &App) -> AudioSettings {
    cx.global::<GlobalSettings>().audio.clone()
}

pub fn discord_settings(cx: &App) -> DiscordSettings {
    cx.global::<GlobalSettings>().discord.clone()
}

pub fn lyrics_settings(cx: &App) -> LyricsSettings {
    cx.global::<GlobalSettings>().lyrics.clone()
}

pub fn queue_settings(cx: &App) -> QueueSettings {
    cx.global::<GlobalSettings>().queue.clone()
}

fn updater(cx: &App) -> Option<Updater> {
    cx.global::<GlobalSettings>().updater.clone()
}

pub fn song_cache_settings(cx: &App) -> CacheLimit {
    cx.global::<GlobalSettings>().song_cache.clone()
}

pub fn cover_cache_settings(cx: &App) -> CacheLimit {
    cx.global::<GlobalSettings>().cover_cache.clone()
}

fn appearance(cx: &App) -> AppearanceSettings {
    cx.global::<GlobalSettings>().appearance.clone()
}

/// Keeps the theme on the system's appearance for as long as the subscription
/// lives, if that is being followed. Only a window reports a change, so each
/// window that can be the last one open holds one.
pub fn follow_system_appearance(window: &Window) -> Subscription {
    window.observe_window_appearance(|window, cx| {
        let appearance = appearance(cx);
        appearance_settings::on_system_appearance_changed(&appearance, window, cx);
    })
}

fn open(cx: &mut App) {
    if let Some(window) = cx.global::<GlobalSettings>().window
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
    let appearance = appearance(cx);
    let audio = audio_settings(cx);
    let song_cache = song_cache_settings(cx);
    let cover_cache = cover_cache_settings(cx);
    let discord = discord_settings(cx);
    let opened = gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| {
            SettingsView::new(
                appearance,
                audio,
                song_cache,
                cover_cache,
                discord,
                window,
                cx,
            )
        })
    });
    if let Ok((window, _)) = opened {
        cx.global_mut::<GlobalSettings>().window = Some(window);
    }
}
