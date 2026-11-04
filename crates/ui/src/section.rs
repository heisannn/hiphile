//! The parts of a screen that scrolls through several kinds of things.

use gpui_kit::component::{
    ActiveTheme as _, Sizable as _, StyledExt as _, button::Button, h_flex, spinner::Spinner,
    v_flex,
};
use gpui_kit::{
    AnyElement, App, InteractiveElement as _, IntoElement, ParentElement as _, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _,
};

use crate::list_navigation::CONTROL_CONTEXT;
use crate::{CoverArt, PlayState, PlayStateIcon};

pub fn section_heading(title: &'static str) -> impl IntoElement {
    div()
        .id(title)
        .role(Role::Heading)
        .aria_level(2)
        .aria_label(title)
        .flex_none()
        .px_6()
        .pt_6()
        .pb_2()
        .font_semibold()
        .child(title)
}

/// A section's content on its way; `label` names the wait for a screen reader.
pub fn section_loading(label: &'static str, cx: &App) -> AnyElement {
    h_flex()
        .id(label)
        .role(Role::ProgressIndicator)
        .aria_label(label)
        .flex_none()
        .h_16()
        .justify_center()
        .child(Spinner::new().color(cx.theme().muted_foreground))
        .into_any_element()
}

/// A failed load of a section's content: why, and a way to try again.
pub fn section_failure(reason: SharedString, retry: Button, cx: &App) -> AnyElement {
    v_flex()
        .flex_none()
        .px_6()
        .gap_2()
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(reason),
        )
        .child(h_flex().key_context(CONTROL_CONTEXT).child(retry))
        .into_any_element()
}

/// What a song's row holds where songs are not all of one album: its cover,
/// its title over `details`, and its duration.
pub fn song_with_cover(
    cover: Option<SharedString>,
    title: SharedString,
    details: impl IntoElement,
    state: Option<PlayState>,
    duration: SharedString,
    cx: &App,
) -> impl IntoElement {
    h_flex()
        .w_full()
        .min_w_0()
        .gap_3()
        .child(CoverArt::new(cover).small())
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(div().text_sm().truncate().child(title))
                .child(details),
        )
        .child(
            // A lane of its own, so the durations stay in line.
            h_flex()
                .w_4()
                .flex_none()
                .justify_end()
                .when_some(state, |this, state| this.child(PlayStateIcon::new(state))),
        )
        .child(
            div()
                .flex_none()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(duration),
        )
}
