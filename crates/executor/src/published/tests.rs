use super::*;

struct Counter {
    shown: u32,
    hidden: u32,
}

impl Publish for Counter {
    type Snapshot = u32;

    fn snapshot(&self) -> &u32 {
        &self.shown
    }
}

fn counter() -> Published<Counter> {
    Published::new(Counter {
        shown: 0,
        hidden: 0,
    })
}

#[test]
fn a_change_is_published_before_update_returns() {
    let counter = counter();
    let mut changes = counter.subscribe();

    counter.update(|counter| counter.shown += 1);
    assert!(changes.has_changed().unwrap());
    assert_eq!(*changes.borrow_and_update(), 1);
    assert_eq!(counter.snapshot(), 1);
}

#[test]
fn a_change_outside_the_snapshot_publishes_nothing() {
    let counter = counter();
    let changes = counter.subscribe();

    counter.update(|counter| counter.hidden += 1);
    assert!(!changes.has_changed().unwrap());
    assert_eq!(counter.read(|counter| counter.hidden), 1);
}
