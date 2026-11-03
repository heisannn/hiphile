//! Feature: the play queue, transport controls, and the now-playing bar.

mod format;
mod player;

use gpui_kit::{App, KeyBinding, actions};

pub use player::Player;

actions!(player, [TogglePlayback, NextSong, PreviousSong]);

/// Registers key bindings. Call before building the menu bar.
///
/// `context` is the app shell's key context: the commands work anywhere in
/// its window, while a focused text field keeps its own use of these keys.
pub fn init(context: &'static str, cx: &mut App) {
    let (next, previous) = if cx.platform_info().is_mac() {
        ("cmd-right", "cmd-left")
    } else {
        ("ctrl-right", "ctrl-left")
    };
    cx.bind_keys([
        KeyBinding::new(next, NextSong, Some(context)),
        KeyBinding::new(previous, PreviousSong, Some(context)),
    ]);
}
