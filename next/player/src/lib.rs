//! The player bar on AntiWebUI, and the colors taken from the cover of what plays.

mod bar;
mod colors;
mod control;
mod cover;
mod position;
mod volume;

pub use bar::{Panels, player_bar};
pub use colors::{cover_seed, seed_of};
pub use cover::{COVER_PIXELS, Cover, now_playing_cover};

#[cfg(any(test, feature = "test-support"))]
pub mod testing;
