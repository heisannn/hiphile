use std::cell::RefCell;
use std::rc::Rc;

use anti_webui::prelude::{create_root, effect};

use super::*;
use crate::testing::{executor, wait_until};

#[test]
fn it_starts_with_the_value_now_and_takes_each_change() {
    let (_runtime, executor) = executor();
    crate::install(executor.clone());
    let (sender, changes) = watch::channel(1);
    let (value, _owner) = create_root(|| follow(changes));
    assert_eq!(value.get_untracked(), 1);

    sender.send(2).unwrap();
    wait_until(|| value.get_untracked() == 2);
}

#[test]
fn quick_changes_arrive_as_the_last_and_never_go_back() {
    let (_runtime, executor) = executor();
    crate::install(executor.clone());
    let (sender, changes) = watch::channel(0);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let (_, _owner) = create_root(|| {
        let value = follow(changes);
        let seen = seen.clone();
        effect(move || seen.borrow_mut().push(value.get()));
    });

    for n in 1..=200 {
        sender.send(n).unwrap();
    }
    wait_until(|| seen.borrow().last() == Some(&200));
    let seen = seen.borrow();
    assert!(seen.windows(2).all(|pair| pair[0] <= pair[1]), "{seen:?}");
    assert!(seen.len() < 10, "{seen:?}");
}

#[test]
fn it_stops_with_its_owner() {
    let (_runtime, executor) = executor();
    crate::install(executor.clone());
    let (sender, changes) = watch::channel(1);
    let (value, owner) = create_root(|| follow(changes));

    owner.dispose();
    wait_until(|| sender.receiver_count() == 0);
    assert!(!value.is_alive());
}
