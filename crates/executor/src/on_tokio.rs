use std::time::Duration;

use tokio::runtime::Handle;
use tokio::task::JoinHandle;

use crate::{BoxFuture, Detach, Spawned, Spawner};

pub(crate) struct Tokio(pub(crate) Handle);

impl Spawner for Tokio {
    fn spawn(&self, future: BoxFuture<'static, ()>) -> Spawned {
        Spawned::new(Aborting(Some(self.0.spawn(future))))
    }

    fn spawn_blocking(&self, work: Box<dyn FnOnce() + Send>) -> Spawned {
        Spawned::new(Aborting(Some(self.0.spawn_blocking(work))))
    }

    fn timer(&self, duration: Duration) -> BoxFuture<'static, ()> {
        // A sleep keeps to the clock of the runtime it was made in, whatever polls it
        let _runtime = self.0.enter();
        Box::pin(tokio::time::sleep(duration))
    }
}

/// Tokio lets a task run on when its handle is dropped.
struct Aborting(Option<JoinHandle<()>>);

impl Drop for Aborting {
    fn drop(&mut self) {
        if let Some(task) = self.0.take() {
            task.abort();
        }
    }
}

impl Detach for Aborting {
    fn detach(mut self: Box<Self>) {
        self.0 = None;
    }
}
