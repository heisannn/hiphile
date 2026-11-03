use gpui_kit::{
    App, Context, FocusHandle, Focusable, InteractiveElement as _, IntoElement, Render,
    Styled as _, Window, div,
};

use crate::HOME_CONTEXT;

/// The home screen. Owns its focus, so window commands reach the app shell
/// while it shows.
pub struct Home {
    focus: FocusHandle,
}

impl Home {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle().tab_stop(true),
        }
    }
}

impl Focusable for Home {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Home {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("home")
            .key_context(HOME_CONTEXT)
            .track_focus(&self.focus)
            .size_full()
    }
}
