use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anti_webui::prelude::{create_root, signal};
use tokio::sync::oneshot;

use super::*;
use crate::testing::{executor, wait_until};

#[test]
fn then_takes_what_the_task_ends_with() {
    let (_runtime, executor) = executor();
    crate::install(executor.clone());
    let got = Rc::new(RefCell::new(None));
    let (_, _owner) = create_root(|| {
        let got = got.clone();
        on_ui(executor.spawn(async { 6 * 7 }), move |n| {
            *got.borrow_mut() = Some(n);
        })
    });

    wait_until(|| *got.borrow() == Some(42));
}

#[test]
fn then_runs_in_the_owner_it_was_asked_in() {
    let (_runtime, executor) = executor();
    crate::install(executor.clone());
    let made = Rc::new(RefCell::new(None));
    let (_, owner) = create_root(|| {
        let made = made.clone();
        on_ui(executor.spawn(async {}), move |()| {
            *made.borrow_mut() = Some(signal(1));
        })
    });
    wait_until(|| made.borrow().is_some());
    let made = made.borrow().unwrap();

    owner.dispose();
    assert!(!made.is_alive());
}

#[test]
fn nothing_is_called_once_the_owner_is_gone_and_the_task_is_called_off() {
    let (_runtime, executor) = executor();
    crate::install(executor.clone());
    let (finish, finished) = oneshot::channel::<()>();
    let called = Rc::new(RefCell::new(false));
    let (_, owner) = create_root(|| {
        let called = called.clone();
        let task = executor.spawn(async move { finished.await.ok() });
        on_ui(task, move |_| *called.borrow_mut() = true)
    });

    owner.dispose();
    wait_until(|| finish.is_closed());
    assert!(!*called.borrow());
}

#[test]
fn a_cancelled_wait_calls_nothing() {
    let (_runtime, executor) = executor();
    crate::install(executor.clone());
    let ran = Arc::new(AtomicBool::new(false));
    let called = Rc::new(RefCell::new(false));
    let (pending, _owner) = create_root(|| {
        let (ran, called) = (ran.clone(), called.clone());
        let task = executor.spawn(async move { ran.store(true, Ordering::SeqCst) });
        on_ui(task, move |()| *called.borrow_mut() = true)
    });
    assert!(pending.is_waiting());

    pending.cancel();
    std::thread::sleep(std::time::Duration::from_millis(20));
    anti_webui::core::channel::pump();
    assert!(!*called.borrow());
}

#[test]
fn waits_in_one_owner_share_its_channel() {
    let (_runtime, executor) = executor();
    crate::install(executor.clone());
    let got = Rc::new(RefCell::new(Vec::new()));
    let (_, _owner) = create_root(|| {
        for n in 0..3 {
            let got = got.clone();
            on_ui(executor.spawn(async move { n }), move |n| {
                got.borrow_mut().push(n);
            });
        }
        let owner = anti_webui::prelude::Owner::current().unwrap();
        assert_eq!(INBOXES.with(|inboxes| inboxes.borrow().len()), 1);
        assert!(INBOXES.with(|inboxes| inboxes.borrow().contains_key(&owner)));
    });

    wait_until(|| got.borrow().len() == 3);
}

#[test]
fn a_scope_waits_in_its_owner_from_outside_any() {
    let (_runtime, executor) = executor();
    crate::install(executor.clone());
    let got = Rc::new(RefCell::new(None));
    let (scope, owner) = create_root(Scope::here);

    {
        let got = got.clone();
        scope.on_ui(executor.spawn(async { 1 }), move |n| {
            *got.borrow_mut() = Some(n)
        });
    }
    wait_until(|| *got.borrow() == Some(1));

    owner.dispose();
    let ran = Arc::new(AtomicBool::new(false));
    let task = executor.spawn({
        let ran = ran.clone();
        async move { ran.store(true, Ordering::SeqCst) }
    });
    assert!(!scope.on_ui(task, |()| {}).is_waiting());
}
