//! How views on AntiWebUI follow the core. The core publishes snapshots on `watch` and notices on
//! `broadcast`, and runs on the executor its host lends it; these turn them into signals and calls
//! on the UI thread, for as long as the owner they were made in lives.
//!
//! Each must be called inside an owner, such as a component's function: without one,
//! `on_cleanup` drops what would stop the work at once, and the view would silently stop
//! following.

use std::cell::RefCell;

use executor::Executor;

mod follow;
mod notices;
mod on_ui;

pub use follow::follow;
pub use notices::notices;
pub use on_ui::{Pending, Scope, on_ui};

thread_local! {
    static EXECUTOR: RefCell<Option<Executor>> = const { RefCell::new(None) };
}

/// Lends this UI thread the executor the core runs on. Call before any view is made.
pub fn install(executor: Executor) {
    EXECUTOR.with(|installed| *installed.borrow_mut() = Some(executor));
}

/// The executor [`install`] lent this thread, for a view that starts work of its own.
pub fn executor() -> Executor {
    EXECUTOR.with(|installed| {
        installed
            .borrow()
            .clone()
            .expect("next_host::install on this thread first")
    })
}

fn expect_owner(what: &str) {
    debug_assert!(
        anti_webui::prelude::Owner::current().is_some(),
        "{what} needs an owner, such as a component's function"
    );
}

#[cfg(test)]
mod testing;
