use std::sync::Mutex;
use std::thread::{self, ThreadId};

use super::*;

fn threads_used(channels: usize, threads: usize, work: usize) -> usize {
    let seen: Mutex<Vec<ThreadId>> = Mutex::new(Vec::new());
    let mut lanes = vec![0usize; channels];
    each(&mut lanes, useful_threads(threads, work), |part| {
        seen.lock().unwrap().push(thread::current().id());
        for lane in part {
            *lane += 1;
        }
    });
    assert!(lanes.iter().all(|lane| *lane == 1));
    let mut seen = seen.into_inner().unwrap();
    seen.sort_unstable_by_key(|id| format!("{id:?}"));
    seen.dedup();
    seen.len()
}

#[test]
fn work_is_shared_out_when_there_is_enough() {
    assert_eq!(threads_used(5, 2, LEAST_PARALLEL_WORK), 2);
    assert_eq!(threads_used(5, 8, LEAST_PARALLEL_WORK), 5);
    assert_eq!(threads_used(5, 2, LEAST_PARALLEL_WORK - 1), 1);
    assert_eq!(threads_used(1, 4, LEAST_PARALLEL_WORK), 1);
    assert_eq!(threads_used(5, 1, usize::MAX), 1);
}
