//! Feature: the play queue, transport controls, the now-playing bar, and the
//! lyrics of what is playing.

mod format;
mod hover_slider;
mod lyrics_view;
mod player;
mod queue_view;
mod transport;

use gpui_kit::{Action, App, Global, KeyBinding, actions};
use ui::list_navigation::bind_list_navigation;

pub use lyrics_view::{LYRICS_CONTEXT, LyricsReveal, LyricsView};
pub use playback::{Placement, RepeatMode};
pub use player::{Panels, Player, PlayerEvent};
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
        /// Goes on from no repeat to all songs, to one song, and back.
        CycleRepeat,
        /// Creates a playlist of the songs in the queue.
        SaveQueue,
        /// Shows the lyrics of the current song, or goes back from them.
        ToggleLyrics,
        /// Goes back from the lyrics.
        CloseLyrics,
        /// Shows or hides the queue, or goes back to it from the lyrics.
        ToggleQueue
    ]
);

/// Repeats as this says from now on.
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

/// The songs kept on this computer, shared by every player in the app.
/// `None` when there is nowhere to keep them or the settings cannot be read.
struct GlobalSongCache(Option<playback::SongCache>);

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
    // amount.
    match capacity {
        Ok(capacity) => set_song_capacity(capacity, cx),
        Err(_) => cx.set_global(GlobalSongCache(None)),
    }

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
        KeyBinding::new(shuffle, ToggleShuffle, Some(context)),
        KeyBinding::new(repeat, CycleRepeat, Some(context)),
        KeyBinding::new(lyrics, ToggleLyrics, Some(context)),
        KeyBinding::new("escape", CloseLyrics, Some(LYRICS_CONTEXT)),
        // Space toggles playback from the lyrics, never from a focused control.
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
        // Space toggles playback from the list, never from a text field.
        KeyBinding::new("space", TogglePlayback, Some(QUEUE_CONTEXT)),
    ]);
    actions::bind_go_to_artist(QUEUE_CONTEXT, cx);
}

/// Keeps at most about `capacity` bytes of songs from now on, or the default
/// amount for `None`. A download still arriving is not kept. Without the cache
/// every song comes from the server, so failing to open it raises no error.
pub fn set_song_capacity(capacity: Option<u64>, cx: &mut App) {
    cx.set_global(GlobalSongCache(playback::SongCache::user(capacity).ok()));
}
