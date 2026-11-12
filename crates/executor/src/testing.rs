use std::collections::VecDeque;
use std::marker::PhantomData;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, ThreadId};
use std::time::Duration;
use std::{future, mem};

use crate::{BoxFuture, Detach, Executor, Spawned, Spawner};

/// Runs what the core spawns on the test's own thread when the test says so, and keeps a clock that
/// moves only when the test moves it. Blocking work runs on that thread too, in its turn.
///
/// A task scheduled from another thread fails the test, as a sign that it does not run
/// deterministically, unless the test has called [`Self::allow_parking`].
///
/// Dropping it drops what its tasks still hold on its thread, so that the last of something, such
/// as a network client, is not dropped on the thread that would have woken the task.
pub struct TestExecutor {
    shared: Arc<Shared>,
    _on_one_thread: PhantomData<Rc<()>>,
}

impl TestExecutor {
    pub fn new() -> Self {
        Self::with_order(Order::Scheduled)
    }

    /// Runs what is ready in an order that `seed` picks, rather than in the order it became ready.
    pub fn with_seed(seed: u64) -> Self {
        Self::with_order(Order::Seeded(seed))
    }

    fn with_order(order: Order) -> Self {
        Self {
            shared: Arc::new(Shared {
                thread: thread::current().id(),
                state: Mutex::new(State {
                    queue: VecDeque::new(),
                    jobs: Vec::new(),
                    alarms: Vec::new(),
                    now: Duration::ZERO,
                    order,
                    allows_parking: false,
                    foreign: None,
                }),
            }),
            _on_one_thread: PhantomData,
        }
    }

    pub fn executor(&self) -> Executor {
        Executor::new(TestSpawner(Arc::downgrade(&self.shared)))
    }

    /// Runs what is ready, and what that makes ready, until nothing is. Neither moves the clock
    /// nor waits for other threads.
    pub fn run_until_parked(&self) {
        while self.tick() {}
    }

    /// Rings the timers that are due or, if none is, runs one task. False if there was neither.
    pub fn tick(&self) -> bool {
        let next = {
            let mut state = lock(&self.shared.state);
            if let Some(foreign) = state.foreign.take() {
                drop(state);
                panic!("{foreign}");
            }
            let due = state
                .alarms
                .partition_point(|alarm| alarm.deadline <= state.now);
            if due > 0 {
                Next::Ring(state.alarms.drain(..due).collect())
            } else if let Some(job) = state.next_job() {
                Next::Run(job)
            } else {
                return false;
            }
        };
        match next {
            Next::Ring(alarms) => alarms.into_iter().for_each(Alarm::ring),
            Next::Run(job) => job.run(),
        }
        true
    }

    /// Moves the clock on by `duration`, stopping at each timer on the way to run what it wakes.
    pub fn advance_clock(&self, duration: Duration) {
        let until = lock(&self.shared.state).now + duration;
        loop {
            self.run_until_parked();
            let mut state = lock(&self.shared.state);
            match state.alarms.first().map(|alarm| alarm.deadline) {
                Some(deadline) if deadline <= until => state.now = deadline,
                _ => {
                    state.now = until;
                    return;
                }
            }
        }
    }

    /// Lets other threads wake tasks, as a test that waits for the network does.
    pub fn allow_parking(&self) {
        lock(&self.shared.state).allows_parking = true;
    }
}

impl Default for TestExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TestExecutor {
    fn drop(&mut self) {
        let foreign = lock(&self.shared.state).foreign.take();
        // Dropping a task's future can spawn or cancel others
        loop {
            let (jobs, queue, alarms) = {
                let mut state = lock(&self.shared.state);
                (
                    mem::take(&mut state.jobs),
                    mem::take(&mut state.queue),
                    mem::take(&mut state.alarms),
                )
            };
            if jobs.is_empty() && queue.is_empty() && alarms.is_empty() {
                break;
            }
            jobs.iter()
                .filter_map(Weak::upgrade)
                .for_each(|job| job.end());
        }
        if let Some(foreign) = foreign
            && !thread::panicking()
        {
            panic!("{foreign}");
        }
    }
}

struct Shared {
    thread: ThreadId,
    state: Mutex<State>,
}

impl Shared {
    fn schedule(&self, job: Arc<Job>) {
        let mut state = lock(&self.state);
        let current = thread::current();
        if current.id() != self.thread && !state.allows_parking && state.foreign.is_none() {
            state.foreign = Some(format!(
                "a task was scheduled on the thread {:?}, not the test's; a test that waits for \
                 another thread calls `allow_parking`",
                current.name().unwrap_or("without a name"),
            ));
        }
        state.queue.push_back(job);
    }
}

struct State {
    queue: VecDeque<Arc<Job>>,
    /// Every job not yet dropped, to end when the executor is dropped.
    jobs: Vec<Weak<Job>>,
    /// By deadline, and those with the same deadline in the order they were set.
    alarms: Vec<Alarm>,
    now: Duration,
    order: Order,
    allows_parking: bool,
    foreign: Option<String>,
}

impl State {
    fn next_job(&mut self) -> Option<Arc<Job>> {
        match &mut self.order {
            Order::Scheduled => self.queue.pop_front(),
            Order::Seeded(_) if self.queue.is_empty() => None,
            Order::Seeded(seed) => {
                let ix = splitmix64(seed) % self.queue.len() as u64;
                self.queue.remove(ix as usize)
            }
        }
    }

    fn enroll(&mut self, job: &Arc<Job>) {
        if self.jobs.len() == self.jobs.capacity() {
            self.jobs.retain(|job| job.strong_count() > 0);
        }
        self.jobs.push(Arc::downgrade(job));
    }
}

enum Order {
    Scheduled,
    Seeded(u64),
}

enum Next {
    Ring(Vec<Alarm>),
    Run(Arc<Job>),
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

struct TestSpawner(Weak<Shared>);

impl Spawner for TestSpawner {
    fn spawn(&self, future: BoxFuture<'static, ()>) -> Spawned {
        let Some(shared) = self.0.upgrade() else {
            return Spawned::new(Handle(None));
        };
        let job = Arc::new(Job {
            shared: self.0.clone(),
            cell: Mutex::new(Cell {
                future: Some(future),
                stage: Stage::Queued,
                is_cancelled: false,
            }),
        });
        lock(&shared.state).enroll(&job);
        shared.schedule(job.clone());
        Spawned::new(Handle(Some(job)))
    }

    fn spawn_blocking(&self, work: Box<dyn FnOnce() + Send>) -> Spawned {
        self.spawn(Box::pin(async move { work() }))
    }

    fn timer(&self, duration: Duration) -> BoxFuture<'static, ()> {
        if duration.is_zero() {
            return Box::pin(future::ready(()));
        }
        let Some(shared) = self.0.upgrade() else {
            return Box::pin(future::pending());
        };
        let bell = Arc::new(Mutex::new(Bell::default()));
        let mut state = lock(&shared.state);
        let deadline = state.now + duration;
        let ix = state
            .alarms
            .partition_point(|alarm| alarm.deadline <= deadline);
        state.alarms.insert(
            ix,
            Alarm {
                deadline,
                bell: bell.clone(),
            },
        );
        Box::pin(Ring(bell))
    }
}

struct Job {
    shared: Weak<Shared>,
    cell: Mutex<Cell>,
}

struct Cell {
    /// Out of the cell while it is polled.
    future: Option<BoxFuture<'static, ()>>,
    stage: Stage,
    is_cancelled: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Idle,
    Queued,
    Running,
    RunningWoken,
    Done,
}

impl Job {
    fn wake_up(self: &Arc<Self>) {
        let mut cell = lock(&self.cell);
        match cell.stage {
            Stage::Idle => cell.stage = Stage::Queued,
            Stage::Running => {
                cell.stage = Stage::RunningWoken;
                return;
            }
            Stage::Queued | Stage::RunningWoken | Stage::Done => return,
        }
        drop(cell);
        self.queue();
    }

    /// Queued rather than dropped at once, as the core cancels under its own locks and dropping
    /// the future may need them.
    fn cancel(self: &Arc<Self>) {
        let mut cell = lock(&self.cell);
        cell.is_cancelled = true;
        if cell.stage != Stage::Idle {
            return;
        }
        cell.stage = Stage::Queued;
        drop(cell);
        self.queue();
    }

    fn queue(self: &Arc<Self>) {
        match self.shared.upgrade() {
            Some(shared) => shared.schedule(self.clone()),
            None => self.end(),
        }
    }

    fn run(self: &Arc<Self>) {
        let mut cell = lock(&self.cell);
        let future = cell.future.take();
        let mut future = match future {
            Some(future) if !cell.is_cancelled => future,
            future => {
                cell.stage = Stage::Done;
                drop(cell);
                drop(future);
                return;
            }
        };
        cell.stage = Stage::Running;
        drop(cell);

        let waker = Waker::from(self.clone());
        let poll = future.as_mut().poll(&mut Context::from_waker(&waker));

        let mut cell = lock(&self.cell);
        if poll.is_ready() || cell.is_cancelled {
            cell.stage = Stage::Done;
            drop(cell);
            drop(future);
            return;
        }
        cell.future = Some(future);
        if cell.stage == Stage::RunningWoken {
            cell.stage = Stage::Queued;
            drop(cell);
            self.queue();
        } else {
            cell.stage = Stage::Idle;
        }
    }

    fn end(&self) {
        let future = {
            let mut cell = lock(&self.cell);
            cell.stage = Stage::Done;
            cell.future.take()
        };
        drop(future);
    }
}

impl Wake for Job {
    fn wake(self: Arc<Self>) {
        self.wake_up();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.wake_up();
    }
}

struct Handle(Option<Arc<Job>>);

impl Drop for Handle {
    fn drop(&mut self) {
        if let Some(job) = self.0.take() {
            job.cancel();
        }
    }
}

impl Detach for Handle {
    fn detach(mut self: Box<Self>) {
        self.0 = None;
    }
}

struct Alarm {
    deadline: Duration,
    bell: Arc<Mutex<Bell>>,
}

impl Alarm {
    fn ring(self) {
        let waker = {
            let mut bell = lock(&self.bell);
            bell.has_rung = true;
            bell.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

#[derive(Default)]
struct Bell {
    has_rung: bool,
    waker: Option<Waker>,
}

struct Ring(Arc<Mutex<Bell>>);

impl Future for Ring {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let mut bell = lock(&self.0);
        if bell.has_rung {
            return Poll::Ready(());
        }
        bell.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests;
