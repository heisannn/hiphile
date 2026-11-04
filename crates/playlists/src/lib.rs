//! Feature: browsing and editing playlists.
//!
//! The server keeps playlists; this feature shows them and changes them there,
//! showing each change at once and saving it behind. Dispatches
//! `actions::PlaySongs`, `actions::EnqueueSongs`, `actions::ShowAlbum` and
//! `actions::ShowArtist`, and
//! keeps the choices that `actions::add_to_playlist_submenu` offers the other
//! features. The app shell routes what they dispatch back to [`Playlists`].

mod entries;
#[cfg(test)]
mod fake_server;
mod filter;
mod name_form;
mod playlists_view;
mod writes;

use gpui_kit::{App, KeyBinding, actions};
use ui::list_navigation::bind_list_navigation;

pub use playlists_view::{Playlists, PlaylistsEvent};

/// Key context of the focused list of playlists.
pub const PLAYLISTS_CONTEXT: &str = "Playlists";
/// Key context of the focused list of a playlist's songs.
pub const SONGS_CONTEXT: &str = "PlaylistSongs";
const ROOT_CONTEXT: &str = "PlaylistsRoot";

actions!(
    playlists,
    [
        /// Playlists: move to its songs. Songs: play from the selection.
        Open,
        /// Plays the playlist from its first song.
        Play,
        /// Playlists: queue it right after the current song. Songs: the
        /// selected song.
        PlayNext,
        /// Playlists: queue it after every other song. Songs: the selected
        /// song.
        AddToQueue,
        /// Songs: return to the list of playlists.
        Back,
        FocusFilter,
        /// Asks for a new name for the playlist.
        Rename,
        /// Asks whether to delete the playlist.
        Delete,
        /// Moves the selected song up the playlist.
        MoveUp,
        /// Moves the selected song down the playlist.
        MoveDown,
        /// Takes the selected song out of the playlist.
        Remove,
        /// Shows the selected song's album on the albums' screen.
        ShowAlbum,
        /// Asks which playlist to add the selected song to.
        AddToPlaylist
    ]
);

/// Registers key bindings. Call before building the menu bar.
pub fn init(cx: &mut App) {
    let (find, play, delete, show_album, add_to_playlist) = if cx.platform_info().is_mac() {
        (
            "cmd-f",
            "cmd-enter",
            "cmd-backspace",
            "cmd-l",
            "cmd-shift-p",
        )
    } else {
        (
            "ctrl-f",
            "ctrl-enter",
            "ctrl-backspace",
            "ctrl-l",
            "ctrl-shift-p",
        )
    };
    for context in [PLAYLISTS_CONTEXT, SONGS_CONTEXT] {
        bind_list_navigation(context, cx);
        cx.bind_keys([
            KeyBinding::new("enter", Open, Some(context)),
            KeyBinding::new("alt-enter", PlayNext, Some(context)),
            KeyBinding::new("shift-enter", AddToQueue, Some(context)),
        ]);
    }
    cx.bind_keys([
        KeyBinding::new("right", Open, Some(PLAYLISTS_CONTEXT)),
        KeyBinding::new(play, Play, Some(PLAYLISTS_CONTEXT)),
        KeyBinding::new("f2", Rename, Some(PLAYLISTS_CONTEXT)),
        KeyBinding::new(delete, Delete, Some(PLAYLISTS_CONTEXT)),
        KeyBinding::new("left", Back, Some(SONGS_CONTEXT)),
        KeyBinding::new("escape", Back, Some(SONGS_CONTEXT)),
        KeyBinding::new("alt-up", MoveUp, Some(SONGS_CONTEXT)),
        KeyBinding::new("alt-down", MoveDown, Some(SONGS_CONTEXT)),
        KeyBinding::new("delete", Remove, Some(SONGS_CONTEXT)),
        KeyBinding::new("backspace", Remove, Some(SONGS_CONTEXT)),
        KeyBinding::new(show_album, ShowAlbum, Some(SONGS_CONTEXT)),
        KeyBinding::new(add_to_playlist, AddToPlaylist, Some(SONGS_CONTEXT)),
        KeyBinding::new(find, FocusFilter, Some(ROOT_CONTEXT)),
    ]);
    actions::bind_go_to_artist(SONGS_CONTEXT, cx);
}
