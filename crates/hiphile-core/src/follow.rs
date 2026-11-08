use std::mem;

use executor::{Executor, Task};
use session::{Readings, ReadingsStatus};
use tokio::sync::broadcast::{self, error::RecvError};
use tokio::sync::watch;

/// Gives `on_change` each new value of `changes`, for as long as the task is kept. Several
/// changes while it runs come as the last one.
pub(crate) fn observe<T: Clone + Send + Sync + 'static>(
    executor: &Executor,
    mut changes: watch::Receiver<T>,
    mut on_change: impl FnMut(T) + Send + 'static,
) -> Task<()> {
    executor.spawn(async move {
        while changes.changed().await.is_ok() {
            // A copy, so that no borrow of the channel is held while calling into a handle, which
            // publishes under its own lock
            let value = changes.borrow_and_update().clone();
            on_change(value);
        }
    })
}

/// Gives `on_notice` each notice sent after `notices` was made, for as long as the task is kept.
/// One that falls behind by more than the channel holds misses the oldest.
pub(crate) fn subscribe<E: Clone + Send + 'static>(
    executor: &Executor,
    mut notices: broadcast::Receiver<E>,
    mut on_notice: impl FnMut(E) + Send + 'static,
) -> Task<()> {
    executor.spawn(async move {
        loop {
            match notices.recv().await {
                Ok(notice) => on_notice(notice),
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => return,
            }
        }
    })
}

/// Calls `on_change` each time the dictionaries come or go, for as long as the task is kept:
/// what was read with them, or for lack of them, is out of date.
pub(crate) fn on_dictionaries_changed(
    executor: &Executor,
    readings: &Readings,
    mut on_change: impl FnMut() + Send + 'static,
) -> Task<()> {
    let changes = readings.subscribe();
    let mut had_dictionaries = changes.borrow().dictionaries().is_some();
    observe(executor, changes, move |status: ReadingsStatus| {
        let has_dictionaries = status.dictionaries().is_some();
        if mem::replace(&mut had_dictionaries, has_dictionaries) != has_dictionaries {
            on_change();
        }
    })
}
