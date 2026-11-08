//! The core runs on threads and a clock that its host lends it through an [`Executor`], and
//! starts no runtime of its own. A GPUI app lends GPUI's background executor (`gpui_host::executor`),
//! so that GPUI's tests drive the core with `run_until_parked` and `advance_clock`. A host on
//! tokio, such as the CLI or a test, lends a runtime with [`Executor::tokio`].

mod on_tokio;
mod published;
mod saver;

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use futures::channel::oneshot;

pub use futures::future::BoxFuture;
pub use published::{Publish, Published};
pub use saver::Saver;

/// What a host implements to lend the core its threads and its clock.
pub trait Spawner: Send + Sync + 'static {
    /// Must not poll `future` before returning: the core spawns while it holds its own locks.
    fn spawn(&self, future: BoxFuture<'static, ()>) -> Spawned;

    /// For work that holds its thread, such as reading the disk. Must not run `work` before
    /// returning.
    fn spawn_blocking(&self, work: Box<dyn FnOnce() + Send>) -> Spawned;

    /// Ready once `duration` has passed on the host's clock, which its tests may move.
    fn timer(&self, duration: Duration) -> BoxFuture<'static, ()>;
}

/// A host's handle on work it was given. Dropping it cancels the work: a future is not polled
/// again, and blocking work that has not started never does. Blocking work under way runs to
/// its end.
pub struct Spawned(Box<dyn Detach>);

impl Spawned {
    pub fn new(handle: impl Detach) -> Self {
        Self(Box::new(handle))
    }
}

/// A host's handle that cancels its work when dropped.
pub trait Detach: Send + 'static {
    /// Lets the work go on without the handle.
    fn detach(self: Box<Self>);
}

#[derive(Clone)]
pub struct Executor(Arc<dyn Spawner>);

impl Executor {
    pub fn new(spawner: impl Spawner) -> Self {
        Self(Arc::new(spawner))
    }

    /// Timers need the runtime's time driver.
    pub fn tokio(runtime: tokio::runtime::Handle) -> Self {
        Self::new(on_tokio::Tokio(runtime))
    }

    pub fn spawn<R: Send + 'static>(
        &self,
        future: impl Future<Output = R> + Send + 'static,
    ) -> Task<R> {
        let (sender, result) = oneshot::channel();
        let spawned = self.0.spawn(Box::pin(async move {
            sender.send(future.await).ok();
        }));
        Task::spawned(result, spawned)
    }

    pub fn spawn_blocking<R: Send + 'static>(
        &self,
        work: impl FnOnce() -> R + Send + 'static,
    ) -> Task<R> {
        let (sender, result) = oneshot::channel();
        let spawned = self.0.spawn_blocking(Box::new(move || {
            sender.send(work()).ok();
        }));
        Task::spawned(result, spawned)
    }

    pub fn timer(&self, duration: Duration) -> Timer {
        Timer(self.0.timer(duration))
    }
}

/// Work given to an [`Executor`], which is cancelled when this is dropped, as a [`Spawned`] is.
#[must_use]
pub struct Task<R> {
    result: oneshot::Receiver<R>,
    spawned: Option<Spawned>,
}

impl<R> Task<R> {
    pub fn ready(value: R) -> Self {
        let (sender, result) = oneshot::channel();
        sender.send(value).ok();
        Self {
            result,
            spawned: None,
        }
    }

    fn spawned(result: oneshot::Receiver<R>, spawned: Spawned) -> Self {
        Self {
            result,
            spawned: Some(spawned),
        }
    }

    pub fn detach(self) {
        if let Some(Spawned(handle)) = self.spawned {
            handle.detach();
        }
    }
}

impl<R> Future for Task<R> {
    type Output = R;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<R> {
        match Pin::new(&mut self.result).poll(cx) {
            Poll::Ready(Ok(value)) => Poll::Ready(value),
            // The host dropped the work unfinished while this handle held it, as it does when
            // the work panics
            Poll::Ready(Err(oneshot::Canceled)) => panic!("a task ended without its result"),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Cancelled when dropped.
#[must_use]
pub struct Timer(BoxFuture<'static, ()>);

impl Future for Timer {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.0.as_mut().poll(cx)
    }
}

#[cfg(test)]
mod tests;
