//! The app shell routes the `actions` this feature dispatches.

mod library_view;

use gpui_kit::{App, KeyBinding, actions};
use ui::list_navigation::bind_list_navigation;

pub use library_view::{Library, LibraryEvent, SyncStage};

pub const ALBUMS_CONTEXT: &str = "LibraryAlbums";
pub const SONGS_CONTEXT: &str = "LibrarySongs";
const LIBRARY_CONTEXT: &str = "Library";

actions!(
    library,
    [
        /// Albums: move to the album's songs. Songs: play from the selection.
        Open,
        Play,
        PlayNext,
        AddToQueue,
        Back,
        FocusFilter,
        Refresh,
        AddToPlaylist
    ]
);

/// Adds the selected album to `playlist`, or to a new one for `None`.
#[derive(Clone, Debug, PartialEq, gpui_kit::Action)]
#[action(namespace = library, no_json)]
struct AddAlbumToPlaylist {
    playlist: Option<subsonic::PlaylistId>,
}

/// Call before building the menu bar.
pub fn init(cx: &mut App) {
    let (find, refresh, add_to_playlist, play) = if cx.platform_info().is_mac() {
        ("cmd-f", "cmd-r", "cmd-p", "cmd-enter")
    } else {
        ("ctrl-f", "ctrl-r", "ctrl-p", "ctrl-enter")
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
        actions::bind_toggle_favorite(context, cx);
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
