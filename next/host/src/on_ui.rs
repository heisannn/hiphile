use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use anti_webui::prelude::{Owner, UiSender, on_cleanup, ui_channel};
use executor::Task;

type Outcome = Box<dyn Any + Send>;

/// What an owner waits for. One per owner rather than a channel per call, as a channel stays
/// registered for as long as its owner lives.
struct Inbox {
    owner: Owner,
    to_ui: UiSender<(u64, Outcome)>,
    waiting: RefCell<HashMap<u64, Waiting>>,
    next_id: Cell<u64>,
}

struct Waiting {
    /// Dropping it calls off the core's task too.
    _forward: Task<()>,
    then: Box<dyn FnOnce(Outcome)>,
}

thread_local! {
    static INBOXES: RefCell<HashMap<Owner, Rc<Inbox>>> = RefCell::default();
}

/// Calls `then` on the UI thread, in the current owner, with what `task` ends with. Nothing is
/// called once the owner is gone, which also calls `task` off. An event handler runs outside any
/// owner, so it waits through a [`Scope`] instead.
pub fn on_ui<R: Send + 'static>(task: Task<R>, then: impl FnOnce(R) + 'static) -> Pending {
    crate::expect_owner("on_ui");
    wait_in(Owner::current().expect("an owner"), task, then)
}

/// The owner a component's function runs in, for its event handlers to wait in.
#[derive(Clone, Copy)]
pub struct Scope(Owner);

impl Scope {
    /// The current owner's.
    pub fn here() -> Self {
        crate::expect_owner("Scope::here");
        Self(Owner::current().expect("an owner"))
    }

    /// [`on_ui`] in this scope's owner. Once the owner is gone, `task` is called off at once.
    pub fn on_ui<R: Send + 'static>(
        &self,
        task: Task<R>,
        then: impl FnOnce(R) + 'static,
    ) -> Pending {
        if !self.0.is_alive() {
            return Pending {
                id: 0,
                inbox: Weak::new(),
            };
        }
        wait_in(self.0, task, then)
    }
}

fn wait_in<R: Send + 'static>(
    owner: Owner,
    task: Task<R>,
    then: impl FnOnce(R) + 'static,
) -> Pending {
    let inbox = inbox(owner);
    let id = inbox.next_id.replace(inbox.next_id.get() + 1);
    let to_ui = inbox.to_ui.clone();
    let forward = crate::executor().spawn(async move {
        let outcome: Outcome = Box::new(task.await);
        to_ui.send((id, outcome));
    });
    inbox.waiting.borrow_mut().insert(
        id,
        Waiting {
            _forward: forward,
            then: Box::new(move |outcome| {
                then(
                    *outcome
                        .downcast::<R>()
                        .expect("sent as what the task ends with"),
                );
            }),
        },
    );
    Pending {
        id,
        inbox: Rc::downgrade(&inbox),
    }
}

fn inbox(owner: Owner) -> Rc<Inbox> {
    if let Some(inbox) = INBOXES.with(|inboxes| inboxes.borrow().get(&owner).cloned()) {
        return inbox;
    }
    // In the owner, so that the channel and the inbox go with it
    owner.with(|| {
        let inbox = Rc::new_cyclic(|this: &Weak<Inbox>| {
            let this = this.clone();
            Inbox {
                owner,
                to_ui: ui_channel(move |(id, outcome): (u64, Outcome)| {
                    let Some(inbox) = this.upgrade() else {
                        return;
                    };
                    let waiting = inbox.waiting.borrow_mut().remove(&id);
                    if let Some(waiting) = waiting {
                        inbox.owner.with(|| (waiting.then)(outcome));
                    }
                }),
                waiting: RefCell::default(),
                next_id: Cell::new(0),
            }
        });
        INBOXES.with(|inboxes| inboxes.borrow_mut().insert(owner, inbox.clone()));
        on_cleanup(move || drop(INBOXES.with(|inboxes| inboxes.borrow_mut().remove(&owner))));
        inbox
    })
}

/// What [`on_ui`] waits for. Dropping it lets the wait go on.
pub struct Pending {
    id: u64,
    inbox: Weak<Inbox>,
}

impl Pending {
    /// Calls the task off, and `then` is never called.
    pub fn cancel(self) {
        if let Some(inbox) = self.inbox.upgrade() {
            let waiting = inbox.waiting.borrow_mut().remove(&self.id);
            drop(waiting);
        }
    }

    pub fn is_waiting(&self) -> bool {
        self.inbox
            .upgrade()
            .is_some_and(|inbox| inbox.waiting.borrow().contains_key(&self.id))
    }
}

#[cfg(test)]
mod tests;
