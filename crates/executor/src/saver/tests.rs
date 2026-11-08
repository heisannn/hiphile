use std::sync::mpsc;
use std::time::Duration;

use tokio::runtime::{Builder, Runtime};

use super::*;

const WAIT: Duration = Duration::from_secs(5);
const NOTHING_MORE: Duration = Duration::from_millis(100);

fn runtime() -> Runtime {
    Builder::new_current_thread().enable_all().build().unwrap()
}

/// Each write says what it is writing, then waits for `open` before it ends.
struct Gated<T> {
    saver: Saver<T>,
    open: mpsc::Sender<()>,
    writing: mpsc::Receiver<T>,
}

fn gated<T: Send + 'static>(runtime: &Runtime) -> Gated<T> {
    let (open, gate) = mpsc::channel();
    let (started, writing) = mpsc::channel();
    let gate = Mutex::new(gate);
    let saver = Saver::new(Executor::tokio(runtime.handle().clone()), move |value| {
        started.send(value).unwrap();
        lock(&gate).recv().unwrap();
    });
    Gated {
        saver,
        open,
        writing,
    }
}

#[test]
fn a_value_given_during_a_write_waits_and_only_the_last_is_written() {
    let runtime = runtime();
    let Gated {
        saver,
        open,
        writing,
    } = gated(&runtime);

    saver.save(1);
    assert_eq!(writing.recv_timeout(WAIT), Ok(1));
    saver.save(2);
    saver.save(3);
    assert!(writing.recv_timeout(NOTHING_MORE).is_err());

    open.send(()).unwrap();
    assert_eq!(writing.recv_timeout(WAIT), Ok(3));
    open.send(()).unwrap();
    assert!(writing.recv_timeout(NOTHING_MORE).is_err());

    saver.save(4);
    assert_eq!(writing.recv_timeout(WAIT), Ok(4));
    open.send(()).unwrap();
}

#[test]
fn save_with_keeps_the_changes_still_waiting() {
    let runtime = runtime();
    let Gated {
        saver,
        open,
        writing,
    } = gated(&runtime);
    let add = |change| {
        move |waiting: Option<Vec<u32>>| {
            let mut changes = waiting.unwrap_or_default();
            changes.push(change);
            changes
        }
    };

    saver.save_with(add(1));
    assert_eq!(writing.recv_timeout(WAIT), Ok(vec![1]));
    saver.save_with(add(2));
    saver.save_with(add(3));

    open.send(()).unwrap();
    assert_eq!(writing.recv_timeout(WAIT), Ok(vec![2, 3]));
    open.send(()).unwrap();
}

#[test]
fn what_was_given_is_written_after_the_saver_is_dropped() {
    let runtime = runtime();
    let Gated {
        saver,
        open,
        writing,
    } = gated(&runtime);

    saver.save(1);
    assert_eq!(writing.recv_timeout(WAIT), Ok(1));
    saver.save(2);
    drop(saver);

    open.send(()).unwrap();
    assert_eq!(writing.recv_timeout(WAIT), Ok(2));
    open.send(()).unwrap();
}

#[test]
fn a_value_taken_while_waiting_is_not_written() {
    let runtime = runtime();
    let Gated {
        saver,
        open,
        writing,
    } = gated(&runtime);

    saver.save(1);
    assert_eq!(writing.recv_timeout(WAIT), Ok(1));
    saver.save(2);
    assert_eq!(saver.take_waiting(), Some(2));
    assert_eq!(saver.take_waiting(), None);

    open.send(()).unwrap();
    assert!(writing.recv_timeout(NOTHING_MORE).is_err());
}
