use gpui_kit::{Context, Task, Window};
use tokio::sync::watch;

/// Notifies the view each time `changes` sees a new value, for as long as the task is kept.
pub fn notify_on_change<V: 'static, T: 'static>(
    mut changes: watch::Receiver<T>,
    cx: &mut Context<V>,
) -> Task<()> {
    cx.spawn(async move |this, cx| {
        while changes.changed().await.is_ok() {
            if this.update(cx, |_, cx| cx.notify()).is_err() {
                break;
            }
        }
    })
}

/// Gives the view each new value of `changes`, as `cx.observe` does for an entity, for as long as
/// the task is kept. Several changes between two turns of the foreground executor come as the
/// last one.
pub fn observe<V: 'static, T: Clone + 'static>(
    mut changes: watch::Receiver<T>,
    cx: &mut Context<V>,
    mut on_change: impl FnMut(&mut V, T, &mut Context<V>) + 'static,
) -> Task<()> {
    cx.spawn(async move |this, cx| {
        while changes.changed().await.is_ok() {
            let value = latest(&mut changes);
            if this
                .update(cx, |this, cx| on_change(this, value, cx))
                .is_err()
            {
                break;
            }
        }
    })
}

/// [`observe`] for a view that needs its window.
pub fn observe_in<V: 'static, T: Clone + 'static>(
    mut changes: watch::Receiver<T>,
    window: &Window,
    cx: &mut Context<V>,
    mut on_change: impl FnMut(&mut V, T, &mut Window, &mut Context<V>) + 'static,
) -> Task<()> {
    cx.spawn_in(window, async move |this, cx| {
        while changes.changed().await.is_ok() {
            let value = latest(&mut changes);
            let changed = this.update_in(cx, |this, window, cx| on_change(this, value, window, cx));
            if changed.is_err() {
                break;
            }
        }
    })
}

/// A copy, so that no borrow of the channel is held while the view calls into the core, which
/// publishes under its own lock.
fn latest<T: Clone>(changes: &mut watch::Receiver<T>) -> T {
    changes.borrow_and_update().clone()
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use gpui_kit::{AppContext as _, IntoElement, Render, TestAppContext, div};

    use super::*;

    struct Follower {
        seen: Vec<u32>,
        _changes: Task<()>,
    }

    impl Render for Follower {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    #[gpui_kit::test]
    fn a_new_value_notifies_the_view(cx: &mut TestAppContext) {
        let (sender, changes) = watch::channel(0);
        let follower = cx.new(|cx| Follower {
            seen: Vec::new(),
            _changes: notify_on_change(changes, cx),
        });
        let notified = Rc::new(Cell::new(0));
        let _observation = cx.update(|cx| {
            let notified = notified.clone();
            cx.observe(&follower, move |_, _| notified.set(notified.get() + 1))
        });

        cx.run_until_parked();
        assert_eq!(notified.get(), 0);
        sender.send_replace(1);
        cx.run_until_parked();
        assert_eq!(notified.get(), 1);
    }

    #[gpui_kit::test]
    fn the_view_sees_the_last_of_the_values_sent_meanwhile(cx: &mut TestAppContext) {
        let (sender, changes) = watch::channel(0);
        let follower = cx.new(|cx| Follower {
            seen: Vec::new(),
            _changes: observe(changes, cx, |this: &mut Follower, value, _| {
                this.seen.push(value)
            }),
        });

        sender.send_replace(1);
        sender.send_replace(2);
        cx.run_until_parked();
        sender.send_replace(3);
        cx.run_until_parked();
        follower.read_with(cx, |it, _| assert_eq!(it.seen, [2, 3]));
    }

    #[gpui_kit::test]
    fn a_view_in_a_window_gets_its_window(cx: &mut TestAppContext) {
        let (sender, changes) = watch::channel(0);
        let seen = Rc::new(RefCell::new(Vec::new()));
        let (_follower, cx) = cx.add_window_view(|window, cx| Follower {
            seen: Vec::new(),
            _changes: observe_in(changes, window, cx, {
                let seen = seen.clone();
                move |_, value, _, _| seen.borrow_mut().push(value)
            }),
        });

        sender.send_replace(1);
        cx.run_until_parked();
        assert_eq!(*seen.borrow(), [1]);
    }
}
