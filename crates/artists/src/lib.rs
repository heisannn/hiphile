//! Feature: browsing artists.
//!
//! A filterable list of artists beside the selected one's top songs, albums and
//! similar artists. The list shows what the saved albums say at once, and the
//! server's answer replaces it. Dispatches `actions::PlaySongs`,
//! `actions::EnqueueSongs`, `actions::ShowAlbum` and the playlist actions; the
//! app shell routes them, and routes `actions::ShowArtist` back to [`Artists`].

mod artists_view;
mod list;

use gpui_kit::{App, KeyBinding, actions};
use ui::list_navigation::bind_list_navigation;

pub use artists_view::{Artists, ArtistsEvent};

/// Key context of the focused list of artists.
pub const ARTISTS_CONTEXT: &str = "Artists";
/// Key context of the focused detail of an artist.
pub const DETAIL_CONTEXT: &str = "ArtistDetail";
const ROOT_CONTEXT: &str = "ArtistsRoot";

actions!(
    artists,
    [
        /// Artists: move to the detail. Detail: play the song, or show the
        /// album or the artist selected.
        Open,
        /// Plays the selection.
        Play,
        /// Queues the selection right after the current song.
        PlayNext,
        /// Queues the selection after every other song.
        AddToQueue,
        /// Detail: return to the list of artists.
        Back,
        FocusFilter,
        /// Asks which playlist to add the selection to.
        AddToPlaylist,
        /// Shows the selected album, or the selected song's, on the albums'
        /// screen.
        GoToAlbum,
        SelectLeft,
        SelectRight
    ]
);

/// Adds the selection to `playlist`, or to a new one for `None`.
#[derive(Clone, Debug, PartialEq, gpui_kit::Action)]
#[action(namespace = artists, no_json)]
struct AddSelectionToPlaylist {
    playlist: Option<subsonic::PlaylistId>,
}

/// Registers key bindings. Call before building the menu bar.
pub fn init(cx: &mut App) {
    let (find, play, add_to_playlist, go_to_album) = if cx.platform_info().is_mac() {
        ("cmd-f", "cmd-enter", "cmd-p", "cmd-l")
    } else {
        ("ctrl-f", "ctrl-enter", "ctrl-p", "ctrl-l")
    };
    for context in [ARTISTS_CONTEXT, DETAIL_CONTEXT] {
        bind_list_navigation(context, cx);
        cx.bind_keys([
            KeyBinding::new("enter", Open, Some(context)),
            KeyBinding::new(play, Play, Some(context)),
            KeyBinding::new("alt-enter", PlayNext, Some(context)),
            KeyBinding::new("shift-enter", AddToQueue, Some(context)),
            KeyBinding::new(add_to_playlist, AddToPlaylist, Some(context)),
        ]);
    }
    cx.bind_keys([
        KeyBinding::new("right", Open, Some(ARTISTS_CONTEXT)),
        KeyBinding::new("left", SelectLeft, Some(DETAIL_CONTEXT)),
        KeyBinding::new("right", SelectRight, Some(DETAIL_CONTEXT)),
        KeyBinding::new("escape", Back, Some(DETAIL_CONTEXT)),
        KeyBinding::new(go_to_album, GoToAlbum, Some(DETAIL_CONTEXT)),
        KeyBinding::new(find, FocusFilter, Some(ROOT_CONTEXT)),
    ]);
    actions::bind_go_to_artist(DETAIL_CONTEXT, cx);
}
