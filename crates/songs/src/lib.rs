mod arrange;
mod songs_view;

use gpui_kit::{App, KeyBinding, actions};
use ui::list_navigation::bind_list_navigation;

pub use songs_view::Songs;

pub const SONGS_CONTEXT: &str = "Songs";
const ROOT_CONTEXT: &str = "SongsRoot";

actions!(
    songs,
    [
        /// Plays the songs on show, from the selected one.
        Open,
        PlayNext,
        AddToQueue,
        FocusFilter,
        AddToPlaylist,
        GoToAlbum
    ]
);

/// Call before building the menu bar.
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
