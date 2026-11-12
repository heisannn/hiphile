//! The player bar, the queue and the lyrics on AntiWebUI, and the colors taken from the cover of
//! what plays.

mod bar;
mod colors;
mod control;
mod cover;
mod lyrics_view;
mod panels;
mod position;
mod queue;
mod reading;
mod volume;

pub use bar::player_bar;
pub use colors::{cover_seed, seed_of};
pub use cover::{COVER_PIXELS, Cover, now_playing_cover};
pub use lyrics_view::lyrics_overlay;
pub use panels::{Panels, QUEUE_MIN_WIDTH};
pub use queue::queue;

#[cfg(any(test, feature = "test-support"))]
pub mod testing;
