use std::sync::{Mutex, MutexGuard, PoisonError};

use tokio::sync::watch;

/// A state that shows itself to others through a snapshot of part of it.
pub trait Publish {
    type Snapshot: Clone + PartialEq;

    fn snapshot(&self) -> &Self::Snapshot;
}

/// A state under its lock, which sends its snapshot each time it changes. The order is the state's
/// lock, then the `watch`'s, so a borrow of a receiver must not be held across a call into the
/// owner.
pub struct Published<S: Publish> {
    state: Mutex<S>,
    sender: watch::Sender<S::Snapshot>,
}

impl<S: Publish> Published<S> {
    pub fn new(state: S) -> Self {
        Self {
            sender: watch::Sender::new(state.snapshot().clone()),
            state: Mutex::new(state),
        }
    }

    pub fn snapshot(&self) -> S::Snapshot {
        lock(&self.state).snapshot().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<S::Snapshot> {
        self.sender.subscribe()
    }

    pub fn read<R>(&self, read: impl FnOnce(&S) -> R) -> R {
        read(&lock(&self.state))
    }

    /// Publishes before letting go of the lock, so that an older snapshot cannot follow a newer
    /// one.
    pub fn update<R>(&self, change: impl FnOnce(&mut S) -> R) -> R {
        let mut state = lock(&self.state);
        let result = change(&mut state);
        let snapshot = state.snapshot();
        self.sender.send_if_modified(|published| {
            let is_new = published != snapshot;
            if is_new {
                published.clone_from(snapshot);
            }
            is_new
        });
        result
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests;
