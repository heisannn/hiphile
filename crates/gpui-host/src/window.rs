use gpui_kit::{AsyncWindowContext, Context, WeakEntity, Window};

/// Updates `view` in the window that `cx` was made for, or gives `None` once that window or the
/// view is gone.
///
/// `WeakEntity::update_in` takes the window that drew the view last instead, and fails while that
/// window is closed and no other has drawn the view since. A task that stopped on that failure
/// would stop for good for a view that a second window draws, such as one that comes and goes.
pub fn update_in_window<V: 'static, R>(
    view: &WeakEntity<V>,
    cx: &mut AsyncWindowContext,
    update: impl FnOnce(&mut V, &mut Window, &mut Context<V>) -> R,
) -> Option<R> {
    cx.update(|window, cx| view.update(cx, |view, cx| update(view, window, cx)).ok())
        .ok()
        .flatten()
}
