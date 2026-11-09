mod hover_slider;
mod lyrics_view;
mod mini_player;
mod player;
mod queue_view;
#[cfg(test)]
mod testing;
mod transport;

use gpui_kit::{Action, App, KeyBinding, actions};
use ui::list_navigation::bind_list_navigation;

use mini_player::MINI_PLAYER_CONTEXT;

pub use lyrics_view::{LYRICS_CONTEXT, LyricsReveal, LyricsView};
pub use mini_player::MiniPlayer;
pub use playback::{Placement, RepeatMode};
pub use player::{Panels, Player};
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
        ToggleMute,
        ToggleShuffle,
        CycleRepeat,
        SaveQueue,
        ToggleLyrics,
        CloseLyrics,
        /// Shows or hides the queue, or goes back to it from the lyrics.
        ToggleQueue,
        /// Opens the mini player, or closes it if it is open.
        ToggleMiniPlayer,
        /// Closes the mini player from its own window. Apart from the toggle, whose
        /// menu item would otherwise show this key.
        CloseMiniPlayer
    ]
);

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = player, no_json)]
pub struct SetRepeat {
    repeat: RepeatMode,
}

impl SetRepeat {
    pub fn new(repeat: RepeatMode) -> Self {
        Self { repeat }
    }

    pub fn repeat(&self) -> RepeatMode {
        self.repeat
    }
}

/// Call before building the menu bar.
///
/// `context` is the app shell's key context: the commands work anywhere in
/// its window, while a focused text field keeps its own use of these keys.
pub fn init(context: &'static str, cx: &mut App) {
    bind_commands(context, cx);
}

fn bind_commands(context: &'static str, cx: &mut App) {
    let [
        next,
        previous,
        forward,
        backward,
        up,
        down,
        mute,
        shuffle,
        repeat,
        lyrics,
        add_to_playlist,
        mini_player,
        close_window,
    ] = if cx.platform_info().is_mac() {
        [
            "cmd-right",
            "cmd-left",
            "cmd-shift-right",
            "cmd-shift-left",
            "cmd-up",
            "cmd-down",
            "cmd-alt-down",
            "cmd-alt-s",
            "cmd-alt-r",
            "cmd-l",
            "cmd-p",
            "cmd-alt-m",
            "cmd-w",
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
            "ctrl-alt-s",
            "ctrl-alt-r",
            "ctrl-l",
            "ctrl-p",
            "ctrl-alt-m",
            "ctrl-w",
        ]
    };
    for context in [context, MINI_PLAYER_CONTEXT] {
        cx.bind_keys([
            KeyBinding::new(next, NextSong, Some(context)),
            KeyBinding::new(previous, PreviousSong, Some(context)),
            KeyBinding::new(forward, SeekForward, Some(context)),
            KeyBinding::new(backward, SeekBackward, Some(context)),
            KeyBinding::new(up, VolumeUp, Some(context)),
            KeyBinding::new(down, VolumeDown, Some(context)),
            KeyBinding::new(mute, ToggleMute, Some(context)),
            KeyBinding::new(shuffle, ToggleShuffle, Some(context)),
            KeyBinding::new(repeat, CycleRepeat, Some(context)),
            KeyBinding::new(mini_player, ToggleMiniPlayer, Some(context)),
        ]);
    }
    cx.bind_keys([
        KeyBinding::new(lyrics, ToggleLyrics, Some(context)),
        KeyBinding::new(close_window, CloseMiniPlayer, Some(MINI_PLAYER_CONTEXT)),
        // Space toggles playback from the window, never from a focused control
        KeyBinding::new("space", TogglePlayback, Some(MINI_PLAYER_CONTEXT)),
        KeyBinding::new("escape", CloseLyrics, Some(LYRICS_CONTEXT)),
        // Space toggles playback from the lyrics, never from a focused control
        KeyBinding::new("space", TogglePlayback, Some(LYRICS_CONTEXT)),
    ]);

    bind_list_navigation(QUEUE_CONTEXT, cx);
    cx.bind_keys([
        KeyBinding::new("enter", queue_view::Play, Some(QUEUE_CONTEXT)),
        KeyBinding::new("alt-enter", queue_view::PlayNext, Some(QUEUE_CONTEXT)),
        KeyBinding::new("delete", queue_view::Remove, Some(QUEUE_CONTEXT)),
        KeyBinding::new("backspace", queue_view::Remove, Some(QUEUE_CONTEXT)),
        KeyBinding::new("alt-up", queue_view::MoveUp, Some(QUEUE_CONTEXT)),
        KeyBinding::new("alt-down", queue_view::MoveDown, Some(QUEUE_CONTEXT)),
        KeyBinding::new(
            add_to_playlist,
            queue_view::AddToPlaylist,
            Some(QUEUE_CONTEXT),
        ),
        // Space toggles playback from the list, never from a text field
        KeyBinding::new("space", TogglePlayback, Some(QUEUE_CONTEXT)),
    ]);
    actions::bind_go_to_artist(QUEUE_CONTEXT, cx);
    actions::bind_toggle_favorite(QUEUE_CONTEXT, cx);
    mini_player::init(cx);
}
