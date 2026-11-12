use anti_webui::prelude::{on_cleanup, ui_channel};
use tokio::sync::broadcast::{Receiver, error::RecvError};

/// Calls `on_notice` on the UI thread with each notice sent after `notices` was made. A view that
/// falls behind by more than the channel holds misses the oldest.
///
/// Notices of different channels reach the UI thread in the order the channels were made, not in
/// the order they were sent, so a handler that needs the state reads the core's `snapshot()`
/// rather than a signal from [`crate::follow`].
pub fn notices<T: Clone + Send + 'static>(
    mut notices: Receiver<T>,
    on_notice: impl Fn(T) + 'static,
) {
    crate::expect_owner("notices");
    let to_ui = ui_channel(on_notice);
    let task = crate::executor().spawn(async move {
        loop {
            match notices.recv().await {
                Ok(notice) => to_ui.send(notice),
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => return,
            }
        }
    });
    on_cleanup(move || drop(task));
}
