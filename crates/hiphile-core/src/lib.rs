//! What the app keeps and decides apart from drawing it, so that any UI can show it and the CLI
//! can share it. Views keep only what is theirs: focus, selection, scroll, drags and animation.

pub mod alerts;
pub mod artists;
mod covers;
#[cfg(feature = "desktop")]
pub mod desktop;
pub mod favorites;
mod follow;
pub mod format;
mod hiphile;
pub mod home;
pub mod library;
pub mod navigation;
pub mod playlists;
pub mod search;
pub mod selection;
pub mod settings;
mod shelf;
mod shown;
pub mod songs;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
#[cfg(test)]
mod testing;
mod workspace;

pub use hiphile::{Caches, Hiphile, HiphileSnapshot, SignInRefused, SwitchFailed};
pub use session::CoverSource;
pub use shelf::Shelf;
pub use shown::Shown;
pub use workspace::Workspace;
