//! Domain-aware components shared by feature crates.

mod cover_art;
mod cover_cache;
mod duration;

pub use cover_art::CoverArt;
pub use cover_cache::CoverCache;
pub use duration::format_duration;
