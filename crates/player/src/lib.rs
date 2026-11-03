//! Feature: the play queue, transport controls, and the now-playing bar.

mod format;
mod player;

use gpui_kit::{App, Global, KeyBinding, actions};

pub use player::{Player, PlayerEvent};

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
}
