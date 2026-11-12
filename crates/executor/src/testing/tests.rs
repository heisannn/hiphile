use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::SeqCst};

use futures::FutureExt as _;
use futures::channel::oneshot;

use super::*;

#[test]
fn run_until_parked_runs_what_is_spawned() {
    let cx = TestExecutor::new();
    let executor = cx.executor();
    let result = Arc::new(AtomicU32::new(0));
    executor
        .spawn({
            let (executor, result) = (executor.clone(), result.clone());
            async move {
                let value = executor.spawn_blocking(|| 6 * 7).await;
                result.store(value, SeqCst);
            }
        })
        .detach();

    cx.run_until_parked();
    assert_eq!(result.load(SeqCst), 42);
}

#[test]
fn nothing_runs_before_the_test_runs_it() {
    let cx = TestExecutor::new();
    let executor = cx.executor();
    let count = Arc::new(AtomicU32::new(0));

    let _task = executor.spawn({
        let count = count.clone();
        async move {
            count.fetch_add(1, SeqCst);
        }
    });
    let _work = executor.spawn_blocking({
        let count = count.clone();
        move || count.fetch_add(1, SeqCst)
    });
    assert_eq!(count.load(SeqCst), 0);

    cx.run_until_parked();
    assert_eq!(count.load(SeqCst), 2);
}

#[test]
fn blocking_work_runs_on_the_test_s_thread() {
    let cx = TestExecutor::new();
    let ran_on = Arc::new(Mutex::new(None));
    let _work = cx.executor().spawn_blocking({
        let ran_on = ran_on.clone();
        move || *lock(&ran_on) = Some(thread::current().id())
    });

    cx.run_until_parked();
    assert_eq!(*lock(&ran_on), Some(thread::current().id()));
}

#[test]
fn dropping_a_task_cancels_it() {
    let cx = TestExecutor::new();
    let executor = cx.executor();
    let ran = Arc::new(AtomicBool::new(false));
    let task = executor.spawn({
        let ran = ran.clone();
        async move { ran.store(true, SeqCst) }
    });
    let work = executor.spawn_blocking({
        let ran = ran.clone();
        move || ran.store(true, SeqCst)
    });

    drop((task, work));
    cx.run_until_parked();
    assert!(!ran.load(SeqCst));
}

#[test]
fn a_cancelled_task_s_future_is_dropped_on_the_next_turn() {
    let cx = TestExecutor::new();
    let (alive, mut ended) = oneshot::channel::<()>();
    let task = cx.executor().spawn(async move {
        let _alive = alive;
        future::pending::<()>().await;
    });
    cx.run_until_parked();

    drop(task);
    assert_eq!(ended.try_recv(), Ok(None));
    cx.run_until_parked();
    assert_eq!(ended.try_recv(), Err(oneshot::Canceled));
}

#[test]
fn a_detached_task_runs_on() {
    let cx = TestExecutor::new();
    let (sender, mut received) = oneshot::channel();
    cx.executor()
        .spawn(async move {
            sender.send(()).ok();
        })
        .detach();

    cx.run_until_parked();
    assert_eq!(received.try_recv(), Ok(Some(())));
}

#[test]
fn a_timer_goes_by_the_test_clock() {
    let cx = TestExecutor::new();
    let executor = cx.executor();
    let fired = Arc::new(AtomicBool::new(false));
    let _task = executor.spawn({
        let (executor, fired) = (executor.clone(), fired.clone());
        async move {
            executor.timer(Duration::from_secs(60)).await;
            fired.store(true, SeqCst);
        }
    });

    cx.run_until_parked();
    cx.advance_clock(Duration::from_secs(59));
    cx.run_until_parked();
    assert!(!fired.load(SeqCst));

    cx.advance_clock(Duration::from_secs(1));
    assert!(fired.load(SeqCst));
    assert_eq!(executor.timer(Duration::ZERO).now_or_never(), Some(()));
}

#[test]
fn advancing_the_clock_stops_at_each_timer_on_the_way() {
    let cx = TestExecutor::new();
    let executor = cx.executor();
    let rounds = Arc::new(AtomicU32::new(0));
    let _task = executor.spawn({
        let (executor, rounds) = (executor.clone(), rounds.clone());
        async move {
            for _ in 0..3 {
                executor.timer(Duration::from_secs(10)).await;
                rounds.fetch_add(1, SeqCst);
            }
        }
    });

    cx.advance_clock(Duration::from_secs(29));
    assert_eq!(rounds.load(SeqCst), 2);
    cx.advance_clock(Duration::from_secs(1));
    assert_eq!(rounds.load(SeqCst), 3);
}

#[test]
fn tick_runs_one_task_at_a_time() {
    let cx = TestExecutor::new();
    let executor = cx.executor();
    let count = Arc::new(AtomicU32::new(0));
    for _ in 0..2 {
        executor
            .spawn({
                let count = count.clone();
                async move {
                    count.fetch_add(1, SeqCst);
                }
            })
            .detach();
    }

    assert!(cx.tick());
    assert_eq!(count.load(SeqCst), 1);
    assert!(cx.tick());
    assert_eq!(count.load(SeqCst), 2);
    assert!(!cx.tick());
}

#[test]
fn tasks_run_in_the_order_they_were_scheduled_unless_seeded() {
    fn order(cx: &TestExecutor) -> Vec<u32> {
        let executor = cx.executor();
        let ran = Arc::new(Mutex::new(Vec::new()));
        for n in 0..8 {
            executor
                .spawn({
                    let ran = ran.clone();
                    async move { lock(&ran).push(n) }
                })
                .detach();
        }
        cx.run_until_parked();
        lock(&ran).clone()
    }

    assert_eq!(order(&TestExecutor::new()), (0..8).collect::<Vec<_>>());
    assert_eq!(
        order(&TestExecutor::with_seed(1)),
        order(&TestExecutor::with_seed(1)),
    );
    let orders: HashSet<_> = (0..10)
        .map(|seed| order(&TestExecutor::with_seed(seed)))
        .collect();
    assert!(orders.len() > 1);
}

#[test]
#[should_panic(expected = "not the test's")]
fn a_wake_from_another_thread_fails_the_test() {
    let cx = TestExecutor::new();
    let (sender, received) = oneshot::channel::<()>();
    let _task = cx.executor().spawn(async move {
        received.await.ok();
    });
    cx.run_until_parked();

    thread::spawn(move || sender.send(())).join().unwrap().ok();
    cx.run_until_parked();
}

#[test]
fn a_test_that_allows_parking_is_woken_from_another_thread() {
    let cx = TestExecutor::new();
    cx.allow_parking();
    let (sender, received) = oneshot::channel();
    let task = cx.executor().spawn(async move { received.await.unwrap() });
    cx.run_until_parked();

    thread::spawn(move || sender.send(42)).join().unwrap().ok();
    cx.run_until_parked();
    assert_eq!(task.now_or_never(), Some(42));
}

#[test]
fn dropping_the_executor_drops_what_its_tasks_hold_on_its_thread() {
    struct Held(Arc<Mutex<Option<ThreadId>>>);

    impl Drop for Held {
        fn drop(&mut self) {
            *lock(&self.0) = Some(thread::current().id());
        }
    }

    let dropped_on = Arc::new(Mutex::new(None));
    let (sender, received) = oneshot::channel::<()>();
    let cx = TestExecutor::new();
    cx.executor()
        .spawn({
            let held = Held(dropped_on.clone());
            async move {
                let _held = held;
                received.await.ok();
            }
        })
        .detach();
    cx.run_until_parked();

    drop(cx);
    assert_eq!(*lock(&dropped_on), Some(thread::current().id()));
    assert_eq!(sender.send(()), Err(()));
}
