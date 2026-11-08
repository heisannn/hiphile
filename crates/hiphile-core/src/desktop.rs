//! What only a desktop has: the Discord status and the system's media controls. They follow the
//! player as any host would, showing its snapshot and sending it commands; a mobile app does the
//! same with the platform's own.

mod discord;
mod media;

pub use discord::{DiscordNotice, DiscordStatus};
pub use media::{MediaNotice, SystemMediaControls};

use std::time::Duration;

/// How often what the followers show is brought up to date with where the song is.
const REFRESH: Duration = Duration::from_millis(250);
/// Pixel size of the cover they show, the same as the lyrics view's.
const COVER_PIXELS: u32 = 512;
