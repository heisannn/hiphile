//! The sections of a long page, such as Home and an artist: a heading, and what the section shows
//! while it has nothing else to.

use anti_webui::prelude::*;
use anti_webui_md3e::components::indeterminate_circular_progress;
use anti_webui_md3e::typography::TextStyle;

use crate::load_state::retry_button;
use crate::pane::INSET;
use crate::type_scale::{error_text, muted_text, styled_text};

/// Above a heading: what parts it from the section before.
const ABOVE: f32 = 16.0;
/// A heading's line is as high as a button at its end, so that one moves nothing.
const LINE: f32 = 40.0;
/// What a small icon button has between its edge and its icon, which brings it closer to the edge
/// than text for its icon to line up.
const BUTTON_PADDING: f32 = 8.0;
/// Where a section waits.
const LOADING_HEIGHT: f32 = 64.0;

/// A section's heading.
pub fn section_heading(title: impl Into<String>) -> Div {
    heading_line(title.into()).px(INSET)
}

/// A section's heading with a button at its end, such as one that reloads the section.
pub fn section_heading_with(title: impl Into<String>, action: Div) -> Div {
    heading_line(title.into())
        .pl(INSET)
        .pr(INSET - BUTTON_PADDING)
        .justify_between()
        .child(action)
}

fn heading_line(title: String) -> Div {
    div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .min_w(0)
        .h(LINE + ABOVE)
        .pt(ABOVE)
        .child(
            styled_text(TextStyle::TITLE_MEDIUM, title.clone())
                .min_w(0)
                .access(move |a| a.role(Role::Heading).level(2).label(title.clone())),
        )
}

/// What a section shows while it loads. `label` names the wait for assistive technology.
pub fn section_loading(label: impl Into<String>) -> Div {
    let label = label.into();
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(LOADING_HEIGHT)
        .access(move |a| a.role(Role::ProgressBar).label(label.clone()))
        .child(indeterminate_circular_progress())
}

/// What a section shows when it could not load: `reason`, and Retry, which runs `retry`.
pub fn section_failure(reason: impl Into<String>, retry: impl Fn() + 'static) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_none()
        .items_start()
        .gap(8.0_f32)
        .px(INSET)
        .py(8.0_f32)
        .child(error_text(reason.into()))
        .child(retry_button(retry))
}

/// A line in place of what a section has none of.
pub fn section_note(note: impl Into<String>) -> Div {
    muted_text(TextStyle::BODY_MEDIUM, note.into())
        .flex_none()
        .px(INSET)
        .py(8.0_f32)
}

#[cfg(test)]
mod tests;
