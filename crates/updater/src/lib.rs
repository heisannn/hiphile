//! Feature: installing new versions of the app from its GitHub releases.
//!
//! Only the builds that CI compiles with `HIPHILE_SELF_UPDATE` set update themselves, so
//! neither `cargo run` nor a build of someone's own is replaced by a release. [`Updater`]
//! installs a new version while the app runs, and it takes over at the next launch, or at
//! [`restart`].

mod install;
mod updater;

use std::sync::Arc;

use gpui_kit::{App, AppContext as _, Entity, Global};

pub use updater::{UpdateState, Updater};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

struct GlobalUpdater(Option<Entity<Updater>>);

impl Global for GlobalUpdater {}

pub fn init(cx: &mut App) {
    let updater = option_env!("HIPHILE_SELF_UPDATE")
        .and_then(|_| install::Installation::of_this_app())
        .map(|installation| cx.new(|cx| Updater::new(Arc::new(installation), cx)));
    cx.set_global(GlobalUpdater(updater));
}

/// `None` in a build that does not update itself.
pub fn updater(cx: &App) -> Option<Entity<Updater>> {
    cx.try_global::<GlobalUpdater>()?.0.clone()
}

/// Quits and opens the app again, which is then the version installed.
pub fn restart(cx: &mut App) {
    cx.restart();
}
