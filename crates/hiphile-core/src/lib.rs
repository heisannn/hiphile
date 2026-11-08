//! What the app keeps and decides apart from drawing it, so that any UI can show it and the CLI
//! can share it. Views keep only what is theirs: focus, selection, scroll, drags and animation.

pub mod artists;
pub mod favorites;
pub mod home;
pub mod library;
pub mod navigation;
pub mod playlists;
pub mod search;
pub mod songs;
