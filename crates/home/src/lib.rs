//! Feature: the home screen.
//!
//! The way to something to listen to from the moment the app opens: albums to
//! try, picked anew each day, then the songs played last, the albums played most
//! and the albums added last, as the server knows them. Dispatches
//! `actions::ShowAlbum`, `actions::PlaySongs` and `actions::EnqueueSongs`; the
//! app shell routes them.

mod home_view;
mod layout;
mod recent;
mod recommend;

use gpui_kit::{App, KeyBinding, NoAction, actions};
use ui::list_navigation::bind_list_navigation;

pub use home_view::Home;

/// Key context of the focused home screen.
pub const HOME_CONTEXT: &str = "Home";
/// Key context around a control inside the screen, whose own use of Enter and
/// Space comes before the screen's.
const CONTROL_CONTEXT: &str = "HomeControl";

actions!(
    home,
    [
        /// Albums: show it in the library. Songs: play from the selection.
        Open,
        /// Plays the selected album or, from it on, the songs shown.
        Play,
        /// Queues the selection right after the current song.
        PlayNext,
        /// Queues the selection after every other song.
        AddToQueue,
        /// Shows the selected album, or the selected song's, in the library.
        ShowInLibrary,
        SelectLeft,
        SelectRight
    ]
);

/// Registers key bindings. Call before building the menu bar.
pub fn init(cx: &mut App) {
    let (play, show_in_library) = if cx.platform_info().is_mac() {
        ("cmd-enter", "cmd-l")
    } else {
        ("ctrl-enter", "ctrl-l")
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
        KeyBinding::new("enter", NoAction, Some(CONTROL_CONTEXT)),
        KeyBinding::new("space", NoAction, Some(CONTROL_CONTEXT)),
    ]);
}
