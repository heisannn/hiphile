//! Only the builds that CI compiles with `HIPHILE_SELF_UPDATE` set update themselves, so
//! neither `cargo run` nor a build of someone's own is replaced by a release. [`Updater`]
//! installs a new version while the app runs, and it takes over when the app next starts,
//! which the host may do at once by restarting it.

mod install;
mod updater;

pub use updater::{Status, UpdateState, Updater};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
