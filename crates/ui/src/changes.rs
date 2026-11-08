use gpui_kit::{Context, Task};
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

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use gpui_kit::{AppContext as _, TestAppContext};

    use super::*;

    struct Follower {
        _changes: Task<()>,
    }

    #[gpui_kit::test]
    fn a_new_value_notifies_the_view(cx: &mut TestAppContext) {
        let (sender, changes) = watch::channel(0);
        let follower = cx.new(|cx| Follower {
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
}
