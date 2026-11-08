use std::mem;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::Executor;

/// Writes the values it is given one at a time, on a blocking thread, so that a slow write
/// cannot leave an older value after a newer one. A value given during a write waits for it,
/// and a newer one replaces it. What was given is written even if the saver is dropped first.
pub struct Saver<T> {
    executor: Executor,
    write: Arc<dyn Fn(T) + Send + Sync>,
    queue: Arc<Mutex<Queue<T>>>,
}

struct Queue<T> {
    waiting: Option<T>,
    is_writing: bool,
}

impl<T: Send + 'static> Saver<T> {
    pub fn new(executor: Executor, write: impl Fn(T) + Send + Sync + 'static) -> Self {
        Self {
            executor,
            write: Arc::new(write),
            queue: Arc::new(Mutex::new(Queue {
                waiting: None,
                is_writing: false,
            })),
        }
    }

    pub fn save(&self, value: T) {
        self.save_with(|_| value);
    }

    /// Makes the value from the one still waiting, if any. For a value that gathers changes,
    /// which must keep those that have not been written.
    pub fn save_with(&self, value: impl FnOnce(Option<T>) -> T) {
        let mut queue = lock(&self.queue);
        let waiting = queue.waiting.take();
        queue.waiting = Some(value(waiting));
        if mem::replace(&mut queue.is_writing, true) {
            return;
        }
        drop(queue);
        let queue = self.queue.clone();
        let write = self.write.clone();
        self.executor
            .spawn_blocking(move || {
                while let Some(value) = next(&queue) {
                    write(value);
                }
            })
            .detach();
    }
}

fn next<T>(queue: &Mutex<Queue<T>>) -> Option<T> {
    let mut queue = lock(queue);
    let value = queue.waiting.take();
    queue.is_writing = value.is_some();
    value
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests;
