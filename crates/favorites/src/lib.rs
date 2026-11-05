//! Every other feature adds to the favorites and takes from them through
//! `actions::SetFavorite`, which the app shell routes to [`Favorites`], and reads
//! them through `actions::is_favorite`, which [`Favorites`] keeps up to date.

mod favorites_view;
mod lists;

use gpui_kit::{App, KeyBinding, actions};
use ui::list_navigation::bind_list_navigation;

pub use favorites_view::{Favorites, FavoritesEvent};

pub const FAVORITES_CONTEXT: &str = "Favorites";

actions!(
    favorites,
    [
        /// Songs: play from the selection. Albums and artists: show it on its
        /// own screen.
        Open,
        Play,
        PlayNext,
        AddToQueue,
        AddToPlaylist,
        GoToAlbum,
        PreviousSection,
        NextSection
    ]
);

/// Adds the selected song or album to `playlist`, or to a new one for `None`.
#[derive(Clone, Debug, PartialEq, gpui_kit::Action)]
#[action(namespace = favorites, no_json)]
struct AddSelectionToPlaylist {
    playlist: Option<subsonic::PlaylistId>,
}

/// Call before building the menu bar.
pub fn init(cx: &mut App) {
    let (play, add_to_playlist, go_to_album) = if cx.platform_info().is_mac() {
        ("cmd-enter", "cmd-p", "cmd-l")
    } else {
        ("ctrl-enter", "ctrl-p", "ctrl-l")
    };
    bind_list_navigation(FAVORITES_CONTEXT, cx);
    cx.bind_keys([
        KeyBinding::new("enter", Open, Some(FAVORITES_CONTEXT)),
        KeyBinding::new(play, Play, Some(FAVORITES_CONTEXT)),
        KeyBinding::new("alt-enter", PlayNext, Some(FAVORITES_CONTEXT)),
        KeyBinding::new("shift-enter", AddToQueue, Some(FAVORITES_CONTEXT)),
        KeyBinding::new(add_to_playlist, AddToPlaylist, Some(FAVORITES_CONTEXT)),
        KeyBinding::new(go_to_album, GoToAlbum, Some(FAVORITES_CONTEXT)),
        KeyBinding::new("left", PreviousSection, Some(FAVORITES_CONTEXT)),
        KeyBinding::new("right", NextSection, Some(FAVORITES_CONTEXT)),
    ]);
    actions::bind_go_to_artist(FAVORITES_CONTEXT, cx);
    actions::bind_toggle_favorite(FAVORITES_CONTEXT, cx);
}
