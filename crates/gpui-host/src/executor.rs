use std::time::Duration;

use executor::{BoxFuture, Detach, Executor, Spawned, Spawner};
use gpui_kit::{App, BackgroundExecutor, Task};

/// The core's executor on GPUI's background threads. In a test, `run_until_parked` runs what
/// the core spawns and `advance_clock` moves its timers.
pub fn executor(cx: &App) -> Executor {
    Executor::new(Gpui(cx.background_executor().clone()))
}

struct Gpui(BackgroundExecutor);

impl Spawner for Gpui {
    fn spawn(&self, future: BoxFuture<'static, ()>) -> Spawned {
        Spawned::new(GpuiTask(self.0.spawn(future)))
    }

    fn spawn_blocking(&self, work: Box<dyn FnOnce() + Send>) -> Spawned {
        Spawned::new(GpuiTask(self.0.spawn(async move { work() })))
    }

    fn timer(&self, duration: Duration) -> BoxFuture<'static, ()> {
        Box::pin(self.0.timer(duration))
    }
}

struct GpuiTask(Task<()>);

impl Detach for GpuiTask {
    fn detach(self: Box<Self>) {
        self.0.detach();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    use gpui_kit::TestAppContext;

    use super::*;

    #[gpui_kit::test]
    fn run_until_parked_runs_what_is_spawned(cx: &mut TestAppContext) {
        let executor = cx.update(|cx| executor(cx));
        let result = Arc::new(AtomicU32::new(0));
        executor
            .spawn({
                let (executor, result) = (executor.clone(), result.clone());
                async move {
                    let value = executor.spawn_blocking(|| 6 * 7).await;
                    result.store(value, Ordering::SeqCst);
                }
            })
            .detach();

        cx.run_until_parked();
        assert_eq!(result.load(Ordering::SeqCst), 42);
    }

    #[gpui_kit::test]
    fn dropping_a_task_cancels_it(cx: &mut TestAppContext) {
        let executor = cx.update(|cx| executor(cx));
        let ran = Arc::new(AtomicBool::new(false));
        let task = executor.spawn({
            let ran = ran.clone();
            async move { ran.store(true, Ordering::SeqCst) }
        });

        drop(task);
        cx.run_until_parked();
        assert!(!ran.load(Ordering::SeqCst));
    }

    #[gpui_kit::test]
    fn a_timer_goes_by_the_test_clock(cx: &mut TestAppContext) {
        let executor = cx.update(|cx| executor(cx));
        let fired = Arc::new(AtomicBool::new(false));
        let _task = executor.spawn({
            let (executor, fired) = (executor.clone(), fired.clone());
            async move {
                executor.timer(Duration::from_secs(60)).await;
                fired.store(true, Ordering::SeqCst);
            }
        });

        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_secs(59));
        cx.run_until_parked();
        assert!(!fired.load(Ordering::SeqCst));

        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        assert!(fired.load(Ordering::SeqCst));
    }
}
