use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::future;
use tokio::runtime::{Builder, Handle, Runtime};

use super::*;

fn runtime() -> Runtime {
    Builder::new_current_thread().enable_all().build().unwrap()
}

#[test]
fn a_task_gives_its_result() {
    let runtime = runtime();
    let executor = Executor::tokio(runtime.handle().clone());

    assert_eq!(runtime.block_on(executor.spawn(async { 6 * 7 })), 42);
    assert_eq!(runtime.block_on(executor.spawn_blocking(|| 6 * 7)), 42);
    assert_eq!(runtime.block_on(Task::ready(42)), 42);
}

#[test]
fn dropping_a_task_cancels_it() {
    let runtime = runtime();
    let executor = Executor::tokio(runtime.handle().clone());
    let (alive, ended) = oneshot::channel::<()>();
    let task = executor.spawn(async move {
        let _alive = alive;
        future::pending::<()>().await;
    });

    drop(task);
    assert_eq!(runtime.block_on(ended), Err(oneshot::Canceled));
}

#[test]
fn a_detached_task_runs_on() {
    let runtime = runtime();
    let executor = Executor::tokio(runtime.handle().clone());
    let (sender, received) = oneshot::channel();

    executor
        .spawn(async move {
            tokio::task::yield_now().await;
            sender.send(()).ok();
        })
        .detach();
    assert_eq!(runtime.block_on(received), Ok(()));
}

#[tokio::test(start_paused = true)]
async fn a_timer_goes_by_the_runtime_s_paused_clock() {
    let executor = Executor::tokio(Handle::current());
    let fired = Arc::new(AtomicBool::new(false));
    let timer = executor.timer(Duration::from_secs(60));
    let task = executor.spawn({
        let fired = fired.clone();
        async move {
            timer.await;
            fired.store(true, Ordering::SeqCst);
        }
    });

    tokio::time::advance(Duration::from_secs(59)).await;
    tokio::task::yield_now().await;
    assert!(!fired.load(Ordering::SeqCst));

    tokio::time::advance(Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    assert!(fired.load(Ordering::SeqCst));
    task.await;
}
