//! The screen of the favorites, which `hiphile_core::favorites::FavoritesState` keeps. The other
//! screens add to them and take from them through `Command::ToggleFavorite`, and read them through
//! `Known`.

mod favorites_view;

pub use favorites_view::{FAVORITES_CONTEXT, FavoritesCommand, favorites};
