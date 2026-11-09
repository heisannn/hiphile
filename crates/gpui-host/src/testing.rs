//! What the tests of several modules share.

use gpui_kit::{AnyView, Context, IntoElement, Render, TestAppContext, Window};

struct Elsewhere(AnyView);

impl Render for Elsewhere {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.0.clone()
    }
}

/// Draws `view` in a window of its own and closes that window, so that the window that last drew
/// the view is gone.
pub(crate) fn draw_in_closed_window(view: impl Into<AnyView>, cx: &mut TestAppContext) {
    let view = view.into();
    let window = cx.add_window(|_, _| Elsewhere(view));
    cx.run_until_parked();
    window
        .update(cx, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
}
