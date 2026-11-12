use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anti_webui::prelude::{Signal, on_cleanup, signal, ui_channel};
use tokio::sync::watch;

/// A signal that holds the latest value of `changes`.
///
/// Only the news of a change crosses to the UI thread, which then reads the latest value: changes
/// that come quickly arrive as the last of them, never go back to an older one, and do not queue
/// up while the UI thread is busy.
pub fn follow<T: Clone + Send + Sync + 'static>(mut changes: watch::Receiver<T>) -> Signal<T> {
    crate::expect_owner("follow");
    let latest = changes.clone();
    let value = signal(changes.borrow_and_update().clone());
    let told = Arc::new(AtomicBool::new(false));
    let to_ui = ui_channel({
        let told = told.clone();
        move |()| {
            // What another channel was told first, in the same round, may have taken the owner away
            if !value.is_alive() {
                return;
            }
            // Cleared before reading, so that a change after the read is told again
            told.store(false, Ordering::SeqCst);
            // A copy first: setting runs the view, which may call into the core, and the core
            // publishes under the lock that a borrow holds
            let now = latest.borrow().clone();
            value.set(now);
        }
    });
    let task = crate::executor().spawn(async move {
        while changes.changed().await.is_ok() {
            changes.mark_unchanged();
            if !told.swap(true, Ordering::SeqCst) {
                to_ui.send(());
            }
        }
    });
    on_cleanup(move || drop(task));
    value
}

#[cfg(test)]
mod tests;
