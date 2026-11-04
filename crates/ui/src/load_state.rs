use std::sync::Arc;

use gpui_kit::component::{ActiveTheme as _, StyledExt as _, button::Button, h_flex, v_flex};
use gpui_kit::{
    App, Div, InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce, SharedString,
    Styled as _, Window, div,
};

use crate::list_navigation::CONTROL_CONTEXT;

/// Something on a screen that comes from the server, which stays on show while
/// a reload that replaces it is on its way or fails.
pub enum Shelf<T> {
    Loading,
    Loaded(Arc<[T]>),
    Failed(SharedString),
}

impl<T> Shelf<T> {
    pub fn items(&self) -> &[T] {
        match self {
            Self::Loaded(items) => items,
            Self::Loading | Self::Failed(_) => &[],
        }
    }

    /// Takes in what a load found. A failure leaves items on show in place.
    pub fn finish(&mut self, result: Result<Vec<T>, SharedString>) {
        match result {
            Ok(items) => *self = Self::Loaded(items.into()),
            Err(_) if matches!(self, Self::Loaded(_)) => {}
            Err(error) => *self = Self::Failed(error),
        }
    }

    /// Shows the load starting, unless there are items to keep on show.
    pub fn begin(&mut self) {
        if !matches!(self, Self::Loaded(_)) {
            *self = Self::Loading;
        }
    }
}

/// `child` in the middle of the room there is, for what stands in for a list
/// or a detail with nothing to show.
pub fn centered(child: impl IntoElement) -> Div {
    div()
        .flex_1()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .p_6()
        .child(child)
}

/// Word of why there is nothing to show.
pub fn muted_text(text: &'static str, cx: &App) -> impl IntoElement {
    div()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text)
}

/// A failed load in place of what it was to show: what failed, why, and a way
/// to try again.
#[derive(IntoElement)]
pub struct LoadFailure {
    title: &'static str,
    reason: SharedString,
    retry: Button,
}

impl LoadFailure {
    pub fn new(title: &'static str, reason: SharedString, retry: Button) -> Self {
        Self {
            title,
            reason,
            retry,
        }
    }
}

impl RenderOnce for LoadFailure {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        v_flex()
            .size_full()
            .p_6()
            .gap_3()
            .child(div().font_medium().child(self.title))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.reason),
            )
            .child(h_flex().key_context(CONTROL_CONTEXT).child(self.retry))
    }
}
