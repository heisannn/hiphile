//! Feature: the home screen.
//!
//! The way to something to listen to from the moment the app opens: albums to
//! try, picked anew each day, then the songs played last, the albums played most
//! and the albums added last, as the server knows them. Dispatches
//! `actions::ShowAlbum`, `actions::ShowArtist`, `actions::PlaySongs` and `actions::EnqueueSongs`; the
//! app shell routes them.

mod home_view;
mod layout;
mod recent;
mod recommend;

use gpui_kit::{App, KeyBinding, actions};
use ui::list_navigation::bind_list_navigation;

pub use home_view::Home;

/// Key context of the focused home screen.
pub const HOME_CONTEXT: &str = "Home";

actions!(
    home,
    [
        /// Albums: show it on the albums' screen. Songs: play from the selection.
        Open,
        /// Plays the selected album or, from it on, the songs shown.
        Play,
        /// Queues the selection right after the current song.
        PlayNext,
        /// Queues the selection after every other song.
        AddToQueue,
        /// Shows the selected album, or the selected song's, on the albums' screen.
        ShowInLibrary,
        /// Asks which playlist to add the selection to.
        ChoosePlaylist,
        SelectLeft,
        SelectRight
    ]
);

/// Adds the selection to `playlist`, or to a new one for `None`.
#[derive(Clone, Debug, PartialEq, gpui_kit::Action)]
#[action(namespace = home, no_json)]
struct AddSelectionToPlaylist {
    playlist: Option<subsonic::PlaylistId>,
}

/// Registers key bindings. Call before building the menu bar.
pub fn init(cx: &mut App) {
    let (play, show_in_library, choose_playlist) = if cx.platform_info().is_mac() {
        ("cmd-enter", "cmd-l", "cmd-shift-p")
    } else {
        ("ctrl-enter", "ctrl-l", "ctrl-shift-p")
    };
    bind_list_navigation(HOME_CONTEXT, cx);
    cx.bind_keys([
        KeyBinding::new("left", SelectLeft, Some(HOME_CONTEXT)),
        KeyBinding::new("right", SelectRight, Some(HOME_CONTEXT)),
        KeyBinding::new("enter", Open, Some(HOME_CONTEXT)),
        KeyBinding::new(play, Play, Some(HOME_CONTEXT)),
        KeyBinding::new("alt-enter", PlayNext, Some(HOME_CONTEXT)),
        KeyBinding::new("shift-enter", AddToQueue, Some(HOME_CONTEXT)),
        KeyBinding::new(show_in_library, ShowInLibrary, Some(HOME_CONTEXT)),
        KeyBinding::new(choose_playlist, ChoosePlaylist, Some(HOME_CONTEXT)),
    ]);
    actions::bind_go_to_artist(HOME_CONTEXT, cx);
}
