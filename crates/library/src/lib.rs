//! Feature: browsing albums and their songs.
//!
//! Shows the server's library saved on this computer at once and brings it
//! up to date in the background. Dispatches `actions::PlaySongs`,
//! `actions::EnqueueSongs` and `actions::ShowArtist`; the app shell routes them.

mod filter;
mod library_view;

use gpui_kit::{App, KeyBinding, actions};
use ui::list_navigation::bind_list_navigation;

pub use library_view::{Library, LibraryEvent};

/// Key context of the focused album list.
pub const ALBUMS_CONTEXT: &str = "LibraryAlbums";
/// Key context of the focused song list.
pub const SONGS_CONTEXT: &str = "LibrarySongs";
const LIBRARY_CONTEXT: &str = "Library";

actions!(
    library,
    [
        /// Albums: move to the album's songs. Songs: play from the selection.
        Open,
        /// Albums: plays the selected album.
        Play,
        /// Albums: queue the album right after the current song. Songs: the
        /// selected song.
        PlayNext,
        /// Albums: queue the album after every other song. Songs: the
        /// selected song.
        AddToQueue,
        /// Songs: return to the album list.
        Back,
        FocusFilter,
        /// Fetches the whole library from the server again.
        Refresh,
        /// Albums: asks which playlist to add the album to. Songs: the
        /// selected song.
        AddToPlaylist
    ]
);

/// Adds the selected album to `playlist`, or to a new one for `None`.
#[derive(Clone, Debug, PartialEq, gpui_kit::Action)]
#[action(namespace = library, no_json)]
struct AddAlbumToPlaylist {
    playlist: Option<subsonic::PlaylistId>,
}

/// Registers key bindings. Call before building the menu bar.
pub fn init(cx: &mut App) {
    let (find, refresh, add_to_playlist, play) = if cx.platform_info().is_mac() {
        ("cmd-f", "cmd-r", "cmd-shift-p", "cmd-enter")
    } else {
        ("ctrl-f", "ctrl-r", "ctrl-shift-p", "ctrl-enter")
    };
    for context in [ALBUMS_CONTEXT, SONGS_CONTEXT] {
        bind_list_navigation(context, cx);
        cx.bind_keys([
            KeyBinding::new("enter", Open, Some(context)),
            KeyBinding::new("alt-enter", PlayNext, Some(context)),
            KeyBinding::new("shift-enter", AddToQueue, Some(context)),
            KeyBinding::new(add_to_playlist, AddToPlaylist, Some(context)),
        ]);
        actions::bind_go_to_artist(context, cx);
    }
    cx.bind_keys([
        KeyBinding::new("right", Open, Some(ALBUMS_CONTEXT)),
        KeyBinding::new(play, Play, Some(ALBUMS_CONTEXT)),
        KeyBinding::new("left", Back, Some(SONGS_CONTEXT)),
        KeyBinding::new("escape", Back, Some(SONGS_CONTEXT)),
        KeyBinding::new(find, FocusFilter, Some(LIBRARY_CONTEXT)),
        KeyBinding::new(refresh, Refresh, Some(LIBRARY_CONTEXT)),
    ]);
}
