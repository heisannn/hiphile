//! Feature: browsing albums and their songs.
//!
//! Dispatches `actions::PlaySongs`; the app shell routes it to the player.

mod filter;
mod library_view;

use gpui_kit::{App, KeyBinding, actions};

pub use library_view::Library;

/// Key context of the focused album list.
pub const ALBUMS_CONTEXT: &str = "LibraryAlbums";
/// Key context of the focused song list.
pub const SONGS_CONTEXT: &str = "LibrarySongs";
const LIBRARY_CONTEXT: &str = "Library";

actions!(
    library,
    [
        SelectNext,
        SelectPrevious,
        SelectFirst,
        SelectLast,
        SelectPageDown,
        SelectPageUp,
        /// Albums: move to the album's songs. Songs: play from the selection.
        Open,
        /// Songs: return to the album list.
        Back,
        FocusFilter
    ]
);

/// Registers key bindings. Call before building the menu bar.
pub fn init(cx: &mut App) {
    let find = if cx.platform_info().is_mac() {
        "cmd-f"
    } else {
        "ctrl-f"
    };
    for context in [ALBUMS_CONTEXT, SONGS_CONTEXT] {
        cx.bind_keys([
            KeyBinding::new("home", SelectFirst, Some(context)),
            KeyBinding::new("end", SelectLast, Some(context)),
            KeyBinding::new("pagedown", SelectPageDown, Some(context)),
            KeyBinding::new("pageup", SelectPageUp, Some(context)),
        ]);
    }
    cx.bind_keys([
        KeyBinding::new("down", SelectNext, Some(ALBUMS_CONTEXT)),
        KeyBinding::new("up", SelectPrevious, Some(ALBUMS_CONTEXT)),
        KeyBinding::new("enter", Open, Some(ALBUMS_CONTEXT)),
        KeyBinding::new("right", Open, Some(ALBUMS_CONTEXT)),
        KeyBinding::new("down", SelectNext, Some(SONGS_CONTEXT)),
        KeyBinding::new("up", SelectPrevious, Some(SONGS_CONTEXT)),
        KeyBinding::new("enter", Open, Some(SONGS_CONTEXT)),
        KeyBinding::new("left", Back, Some(SONGS_CONTEXT)),
        KeyBinding::new("escape", Back, Some(SONGS_CONTEXT)),
        KeyBinding::new(find, FocusFilter, Some(LIBRARY_CONTEXT)),
    ]);
}
