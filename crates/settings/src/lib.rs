//! Each setting saves itself. The app shell hands the settings to those who use them and passes
//! on their change events. The appearance only concerns the theme, which this crate keeps on it.

mod appearance_settings;
mod audio_settings;
mod cache_limit;
mod cache_limit_field;
mod discord_settings;
mod dsd_section;
mod dsd_settings;
mod imgbb_key_field;
mod lyrics_settings;
mod number_field;
mod queue_settings;
mod settings_view;
mod song_cache_usage;

use cache_limit::CacheKind;
use gpui_kit::component::TitleBar;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Entity, Global, KeyBinding, Subscription,
    TitlebarOptions, Window, WindowBounds, WindowOptions, actions, px, size,
};

pub use audio_settings::{AudioSettings, OutputChanged, TransitionsChanged};
pub use cache_limit::{CacheLimit, CacheLimitChanged};
pub use discord_settings::{DiscordSettings, ImgbbKeyChanged, PresenceChanged};
pub use lyrics_settings::{LyricsHighlightChanged, LyricsSettings};
pub use queue_settings::{QueueSettings, SkipInstrumentalsChanged};

use appearance_settings::AppearanceSettings;
use settings_view::SettingsView;

const SETTINGS_CONTEXT: &str = "Settings";

actions!(settings, [OpenSettings, CloseWindow]);

struct GlobalSettings {
    appearance: Entity<AppearanceSettings>,
    audio: Entity<AudioSettings>,
    song_cache: Entity<CacheLimit>,
    cover_cache: Entity<CacheLimit>,
    discord: Entity<DiscordSettings>,
    lyrics: Entity<LyricsSettings>,
    queue: Entity<QueueSettings>,
    /// It may have been closed since.
    window: Option<AnyWindowHandle>,
}

impl Global for GlobalSettings {}

/// Call before building the menu bar.
pub fn init(cx: &mut App) {
    number_field::init(cx);
    let appearance = cx.new(AppearanceSettings::new);
    let audio = cx.new(AudioSettings::new);
    let song_cache = cx.new(|cx| CacheLimit::new(CacheKind::Songs, cx));
    let cover_cache = cx.new(|cx| CacheLimit::new(CacheKind::Covers, cx));
    let discord = cx.new(DiscordSettings::new);
    let lyrics = cx.new(LyricsSettings::new);
    let queue = cx.new(QueueSettings::new);
    cx.set_global(GlobalSettings {
        appearance,
        audio,
        song_cache,
        cover_cache,
        discord,
        lyrics,
        queue,
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

pub fn audio_settings(cx: &App) -> Entity<AudioSettings> {
    cx.global::<GlobalSettings>().audio.clone()
}

pub fn discord_settings(cx: &App) -> Entity<DiscordSettings> {
    cx.global::<GlobalSettings>().discord.clone()
}

pub fn lyrics_settings(cx: &App) -> Entity<LyricsSettings> {
    cx.global::<GlobalSettings>().lyrics.clone()
}

pub fn queue_settings(cx: &App) -> Entity<QueueSettings> {
    cx.global::<GlobalSettings>().queue.clone()
}

pub fn song_cache_settings(cx: &App) -> Entity<CacheLimit> {
    cx.global::<GlobalSettings>().song_cache.clone()
}

pub fn cover_cache_settings(cx: &App) -> Entity<CacheLimit> {
    cx.global::<GlobalSettings>().cover_cache.clone()
}

/// Keeps the theme on the system's appearance for as long as the subscription
/// lives, if that is being followed. Only a window reports a change, so each
/// window that can be the last one open holds one.
pub fn follow_system_appearance(window: &Window) -> Subscription {
    window.observe_window_appearance(|window, cx| {
        let appearance = cx.global::<GlobalSettings>().appearance.clone();
        appearance.update(cx, |appearance, cx| {
            appearance.on_system_appearance_changed(window, cx);
        });
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
    let appearance = cx.global::<GlobalSettings>().appearance.clone();
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
