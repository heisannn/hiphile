use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::{
    AnyElement, App, IntoElement, ParentElement as _, RenderOnce, SharedString, Styled as _,
    Window, div, prelude::FluentBuilder as _,
};

#[derive(IntoElement)]
pub struct RowText {
    title: SharedString,
    details: Option<SharedString>,
    detail_line: Option<AnyElement>,
    dimmed: bool,
}

impl RowText {
    pub fn new(title: SharedString) -> Self {
        Self {
            title,
            details: None,
            detail_line: None,
            dimmed: false,
        }
    }

    pub fn details(mut self, details: impl Into<Option<SharedString>>) -> Self {
        self.details = details.into();
        self
    }

    pub fn detail_line(mut self, line: impl IntoElement) -> Self {
        self.detail_line = Some(line.into_any_element());
        self
    }

    pub fn dimmed(mut self, dimmed: bool) -> Self {
        self.dimmed = dimmed;
        self
    }
}

impl RenderOnce for RowText {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let muted_foreground = cx.theme().muted_foreground;
        v_flex()
            .flex_1()
            .min_w_0()
            .child(
                div()
                    .text_sm()
                    .truncate()
                    .when(self.dimmed, |this| this.text_color(muted_foreground))
                    .child(self.title),
            )
            .when_some(self.details, |this, details| {
                this.child(
                    div()
                        .text_xs()
                        .truncate()
                        .text_color(muted_foreground)
                        .child(details),
                )
            })
            .children(self.detail_line)
    }
}
