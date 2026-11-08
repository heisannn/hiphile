use gpui_kit::{Context, Task, Window};
use tokio::sync::broadcast::{Receiver, error::RecvError};

/// Gives the view each notice sent after `notices` was made, as `cx.subscribe` does for an
/// entity's events, for as long as the task is kept. A view that falls behind by more than the
/// channel holds misses the oldest.
pub fn subscribe<V: 'static, E: Clone + 'static>(
    mut notices: Receiver<E>,
    cx: &mut Context<V>,
    mut on_notice: impl FnMut(&mut V, E, &mut Context<V>) + 'static,
) -> Task<()> {
    cx.spawn(async move |this, cx| {
        while let Some(notice) = next(&mut notices).await {
            if this
                .update(cx, |this, cx| on_notice(this, notice, cx))
                .is_err()
            {
                break;
            }
        }
    })
}

/// [`subscribe`] for a view that needs its window.
pub fn subscribe_in<V: 'static, E: Clone + 'static>(
    mut notices: Receiver<E>,
    window: &Window,
    cx: &mut Context<V>,
    mut on_notice: impl FnMut(&mut V, E, &mut Window, &mut Context<V>) + 'static,
) -> Task<()> {
    cx.spawn_in(window, async move |this, cx| {
        while let Some(notice) = next(&mut notices).await {
            let handled =
                this.update_in(cx, |this, window, cx| on_notice(this, notice, window, cx));
            if handled.is_err() {
                break;
            }
        }
    })
}

/// `None` once every sender is gone.
async fn next<E: Clone>(notices: &mut Receiver<E>) -> Option<E> {
    loop {
        match notices.recv().await {
            Ok(notice) => return Some(notice),
            Err(RecvError::Lagged(_)) => {}
            Err(RecvError::Closed) => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui_kit::{AppContext as _, IntoElement, Render, TestAppContext, div};
    use tokio::sync::broadcast;

    use super::*;

    struct Listener {
        heard: Vec<u32>,
        _notices: Task<()>,
    }

    impl Render for Listener {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    #[gpui_kit::test]
    fn every_notice_reaches_every_view(cx: &mut TestAppContext) {
        let (sender, _) = broadcast::channel(8);
        let listen = |cx: &mut TestAppContext| {
            let notices = sender.subscribe();
            cx.new(|cx| Listener {
                heard: Vec::new(),
                _notices: subscribe(notices, cx, |this: &mut Listener, notice, _| {
                    this.heard.push(notice)
                }),
            })
        };
        let first = listen(cx);
        let second = listen(cx);

        sender.send(1).unwrap();
        sender.send(2).unwrap();
        cx.run_until_parked();
        first.read_with(cx, |it, _| assert_eq!(it.heard, [1, 2]));
        second.read_with(cx, |it, _| assert_eq!(it.heard, [1, 2]));
    }

    #[gpui_kit::test]
    fn a_view_that_falls_behind_goes_on_from_the_oldest_notice_left(cx: &mut TestAppContext) {
        let (sender, notices) = broadcast::channel(2);
        let listener = cx.new(|cx| Listener {
            heard: Vec::new(),
            _notices: subscribe(notices, cx, |this: &mut Listener, notice, _| {
                this.heard.push(notice)
            }),
        });

        for notice in 1..=4 {
            sender.send(notice).unwrap();
        }
        cx.run_until_parked();
        listener.read_with(cx, |it, _| assert_eq!(it.heard, [3, 4]));
    }

    #[gpui_kit::test]
    fn a_view_in_a_window_gets_its_window(cx: &mut TestAppContext) {
        let (sender, notices) = broadcast::channel(8);
        let heard = Rc::new(RefCell::new(Vec::new()));
        let (_listener, cx) = cx.add_window_view(|window, cx| Listener {
            heard: Vec::new(),
            _notices: subscribe_in(notices, window, cx, {
                let heard = heard.clone();
                move |_, notice, _, _| heard.borrow_mut().push(notice)
            }),
        });

        sender.send(1).unwrap();
        cx.run_until_parked();
        assert_eq!(*heard.borrow(), [1]);
    }
}
