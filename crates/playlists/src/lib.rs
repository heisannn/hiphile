//! The screen of the playlists, which `hiphile_core::playlists::PlaylistsState` keeps, with its
//! dialogs and the choice of playlist asked for from the keyboard. The app shell routes what the
//! other features dispatch to [`Playlists`], and keeps the choices that
//! `actions::add_to_playlist_submenu` offers them in step with the state.

mod filter;
mod name_form;
mod playlists_view;

use gpui_kit::{App, KeyBinding, actions};
use ui::list_navigation::bind_list_navigation;

pub use playlists_view::{Playlists, PlaylistsEvent};

pub const PLAYLISTS_CONTEXT: &str = "Playlists";
pub const SONGS_CONTEXT: &str = "PlaylistSongs";
const ROOT_CONTEXT: &str = "PlaylistsRoot";

actions!(
    playlists,
    [
        /// Playlists: move to its songs. Songs: play from the selection.
        Open,
        /// Plays the playlist from its first song.
        Play,
        PlayNext,
        AddToQueue,
        Back,
        FocusFilter,
        Rename,
        Delete,
        MoveUp,
        MoveDown,
        Remove,
        ShowAlbum,
        AddToPlaylist
    ]
);

/// Call before building the menu bar.
pub fn init(cx: &mut App) {
    let (find, play, delete, show_album, add_to_playlist) = if cx.platform_info().is_mac() {
        ("cmd-f", "cmd-enter", "cmd-backspace", "cmd-l", "cmd-p")
    } else {
        ("ctrl-f", "ctrl-enter", "ctrl-backspace", "ctrl-l", "ctrl-p")
    };
    for context in [PLAYLISTS_CONTEXT, SONGS_CONTEXT] {
        bind_list_navigation(context, cx);
        cx.bind_keys([
            KeyBinding::new("enter", Open, Some(context)),
            KeyBinding::new("alt-enter", PlayNext, Some(context)),
            KeyBinding::new("shift-enter", AddToQueue, Some(context)),
        ]);
    }
    cx.bind_keys([
        KeyBinding::new("right", Open, Some(PLAYLISTS_CONTEXT)),
        KeyBinding::new(play, Play, Some(PLAYLISTS_CONTEXT)),
        KeyBinding::new("f2", Rename, Some(PLAYLISTS_CONTEXT)),
        KeyBinding::new(delete, Delete, Some(PLAYLISTS_CONTEXT)),
        KeyBinding::new("left", Back, Some(SONGS_CONTEXT)),
        KeyBinding::new("escape", Back, Some(SONGS_CONTEXT)),
        KeyBinding::new("alt-up", MoveUp, Some(SONGS_CONTEXT)),
        KeyBinding::new("alt-down", MoveDown, Some(SONGS_CONTEXT)),
        KeyBinding::new("delete", Remove, Some(SONGS_CONTEXT)),
        KeyBinding::new("backspace", Remove, Some(SONGS_CONTEXT)),
        KeyBinding::new(show_album, ShowAlbum, Some(SONGS_CONTEXT)),
        KeyBinding::new(add_to_playlist, AddToPlaylist, Some(SONGS_CONTEXT)),
        KeyBinding::new(find, FocusFilter, Some(ROOT_CONTEXT)),
    ]);
    actions::bind_go_to_artist(SONGS_CONTEXT, cx);
    actions::bind_toggle_favorite(SONGS_CONTEXT, cx);
}
