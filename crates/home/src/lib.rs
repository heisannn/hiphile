//! The app shell routes the `actions` this feature dispatches.

mod home_view;
mod layout;
mod recent;
mod recommend;

use gpui_kit::{App, KeyBinding, actions};
use ui::list_navigation::bind_list_navigation;

pub use home_view::Home;

pub const HOME_CONTEXT: &str = "Home";

actions!(
    home,
    [
        /// Albums: show it on the albums' screen. Songs: play from the selection.
        Open,
        /// Plays the selected album or, from it on, the songs shown.
        Play,
        PlayNext,
        AddToQueue,
        ShowInLibrary,
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

/// Call before building the menu bar.
pub fn init(cx: &mut App) {
    let (play, show_in_library, choose_playlist) = if cx.platform_info().is_mac() {
        ("cmd-enter", "cmd-l", "cmd-p")
    } else {
        ("ctrl-enter", "ctrl-l", "ctrl-p")
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
    actions::bind_toggle_favorite(HOME_CONTEXT, cx);
}
