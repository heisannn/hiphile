use std::sync::{Arc, Weak};
use std::time::Duration;

use executor::{Executor, Task};

/// Neither the devices nor the songs kept report a change, so a host that shows them asks again
/// this often.
pub(super) const REFRESH: Duration = Duration::from_secs(2);

/// Keeps what a handle gave fresh for as long as it is held, as a host does while it shows it.
#[must_use = "dropping it stops refreshing"]
pub struct Freshness {
    _refresh: Task<()>,
}

impl Freshness {
    /// Calls `refresh` every [`REFRESH`] while `of` lives.
    pub(super) fn keep<T: Send + Sync + 'static>(
        executor: &Executor,
        of: Weak<T>,
        refresh: impl Fn(Arc<T>) + Send + 'static,
    ) -> Self {
        let timers = executor.clone();
        Self {
            _refresh: executor.spawn(async move {
                loop {
                    timers.timer(REFRESH).await;
                    let Some(of) = of.upgrade() else {
                        return;
                    };
                    refresh(of);
                }
            }),
        }
    }
}
