//! Feature: the play queue, transport controls, and the now-playing bar.

mod format;
mod player;
mod queue_view;

use gpui_kit::{App, Global, KeyBinding, actions};
use ui::list_navigation::bind_list_navigation;

pub use playback::Placement;
pub use player::{Player, PlayerEvent};
pub use queue_view::{QUEUE_CONTEXT, QueueView};

actions!(
    player,
    [
        TogglePlayback,
        NextSong,
        PreviousSong,
        SeekForward,
        SeekBackward,
        VolumeUp,
        VolumeDown,
        ToggleMute
    ]
);

/// The songs kept on this computer, shared by every player in the app.
/// Absent when there is nowhere to keep them, the settings keep none, or
/// the settings cannot be read.
struct GlobalSongCache(playback::SongCache);

impl Global for GlobalSongCache {}

/// Registers key bindings and opens the songs kept on this computer. Call
/// before building the menu bar.
///
/// `context` is the app shell's key context: the commands work anywhere in
/// its window, while a focused text field keeps its own use of these keys.
pub fn init(context: &'static str, cx: &mut App) {
    let capacity = match config::Store::user() {
        Ok(store) => store.song_cache_capacity(),
        // Without a home directory there are no settings to follow.
        Err(_) => Ok(None),
    };
    // A setting that cannot be read keeps no songs rather than the default
    // amount. Without the cache every song comes from the server, so
    // failing to open it raises no error.
    if let Ok(capacity) = capacity
        && let Ok(Some(song_cache)) = playback::SongCache::user(capacity)
    {
        cx.set_global(GlobalSongCache(song_cache));
    }

    let [next, previous, forward, backward, up, down, mute] = if cx.platform_info().is_mac() {
        [
            "cmd-right",
            "cmd-left",
            "cmd-shift-right",
            "cmd-shift-left",
            "cmd-up",
            "cmd-down",
            "cmd-alt-down",
        ]
    } else {
        [
            "ctrl-right",
            "ctrl-left",
            "ctrl-shift-right",
            "ctrl-shift-left",
            "ctrl-up",
            "ctrl-down",
            "ctrl-alt-down",
        ]
    };
    cx.bind_keys([
        KeyBinding::new(next, NextSong, Some(context)),
        KeyBinding::new(previous, PreviousSong, Some(context)),
        KeyBinding::new(forward, SeekForward, Some(context)),
        KeyBinding::new(backward, SeekBackward, Some(context)),
        KeyBinding::new(up, VolumeUp, Some(context)),
        KeyBinding::new(down, VolumeDown, Some(context)),
        KeyBinding::new(mute, ToggleMute, Some(context)),
    ]);

    bind_list_navigation(QUEUE_CONTEXT, cx);
    cx.bind_keys([
        KeyBinding::new("enter", queue_view::Play, Some(QUEUE_CONTEXT)),
        KeyBinding::new("alt-enter", queue_view::PlayNext, Some(QUEUE_CONTEXT)),
        KeyBinding::new("delete", queue_view::Remove, Some(QUEUE_CONTEXT)),
        KeyBinding::new("backspace", queue_view::Remove, Some(QUEUE_CONTEXT)),
        KeyBinding::new("alt-up", queue_view::MoveUp, Some(QUEUE_CONTEXT)),
        KeyBinding::new("alt-down", queue_view::MoveDown, Some(QUEUE_CONTEXT)),
        // Space toggles playback from the list, never from a text field.
        KeyBinding::new("space", TogglePlayback, Some(QUEUE_CONTEXT)),
    ]);
}
