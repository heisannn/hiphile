//! The Playlists screen on AntiWebUI: the playlists that `hiphile_core::playlists::PlaylistsState`
//! keeps, beside the songs of the one chosen, with renaming and deleting them and moving and
//! removing their songs. Making a playlist is the shell's, as any screen can ask for it.

mod playlists_view;

pub use playlists_view::playlists;
