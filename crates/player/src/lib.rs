//! Feature: the play queue, transport controls, and the now-playing bar.

mod format;
mod player;

use gpui_kit::{App, KeyBinding, actions};

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

/// Registers key bindings. Call before building the menu bar.
///
/// `context` is the app shell's key context: the commands work anywhere in
/// its window, while a focused text field keeps its own use of these keys.
pub fn init(context: &'static str, cx: &mut App) {
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
