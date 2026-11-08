//! The app shell routes the `actions` this feature dispatches, and routes
//! `actions::ShowArtist` back to [`Artists`].

mod artists_view;

use gpui_kit::{App, KeyBinding, actions};
use ui::list_navigation::bind_list_navigation;

pub use artists_view::{Artists, ArtistsEvent};

pub const ARTISTS_CONTEXT: &str = "Artists";
pub const DETAIL_CONTEXT: &str = "ArtistDetail";
const ROOT_CONTEXT: &str = "ArtistsRoot";

actions!(
    artists,
    [
        /// Artists: move to the detail. Detail: play the song, or show the
        /// album or the artist selected.
        Open,
        Play,
        PlayNext,
        AddToQueue,
        Back,
        FocusFilter,
        AddToPlaylist,
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

/// Call before building the menu bar.
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
        actions::bind_toggle_favorite(context, cx);
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
