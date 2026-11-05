//! Feature: every song in the library.
//!
//! The songs saved on this computer, or the server's when there are none yet,
//! in a table sorted by the column whose heading was clicked. Dispatches
//! `actions::PlaySongs`, `actions::EnqueueSongs`, `actions::ShowAlbum` and
//! `actions::ShowArtist`; the app shell routes them.

mod arrange;
mod songs_view;

use gpui_kit::{App, KeyBinding, actions};
use ui::list_navigation::bind_list_navigation;

pub use songs_view::Songs;

/// Key context of the focused table of songs.
pub const SONGS_CONTEXT: &str = "Songs";
const ROOT_CONTEXT: &str = "SongsRoot";

actions!(
    songs,
    [
        /// Plays the songs on show, from the selected one.
        Open,
        /// Queues the selected song right after the current song.
        PlayNext,
        /// Queues the selected song after every other song.
        AddToQueue,
        FocusFilter,
        /// Asks which playlist to add the selected song to.
        AddToPlaylist,
        /// Shows the selected song's album on the albums' screen.
        GoToAlbum
    ]
);

/// Registers key bindings. Call before building the menu bar.
pub fn init(cx: &mut App) {
    let (find, add_to_playlist, go_to_album) = if cx.platform_info().is_mac() {
        ("cmd-f", "cmd-p", "cmd-l")
    } else {
        ("ctrl-f", "ctrl-p", "ctrl-l")
    };
    bind_list_navigation(SONGS_CONTEXT, cx);
    cx.bind_keys([
        KeyBinding::new("enter", Open, Some(SONGS_CONTEXT)),
        KeyBinding::new("alt-enter", PlayNext, Some(SONGS_CONTEXT)),
        KeyBinding::new("shift-enter", AddToQueue, Some(SONGS_CONTEXT)),
        KeyBinding::new(add_to_playlist, AddToPlaylist, Some(SONGS_CONTEXT)),
        KeyBinding::new(go_to_album, GoToAlbum, Some(SONGS_CONTEXT)),
        KeyBinding::new(find, FocusFilter, Some(ROOT_CONTEXT)),
    ]);
    actions::bind_go_to_artist(SONGS_CONTEXT, cx);
    actions::bind_toggle_favorite(SONGS_CONTEXT, cx);
}
