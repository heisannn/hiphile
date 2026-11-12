//! What a pane shows in place of what it is for: while that loads, when there is none of it, and
//! when it could not be had.

use anti_webui::prelude::*;
use anti_webui_md3e::components::{ButtonStyle, loading_indicator, styled_button};
use anti_webui_md3e::typography::TextStyle;

use crate::type_scale::{error_text, muted_text, styled_text};

/// Around what stands in the middle of a pane.
const MARGIN: f32 = 32.0;
/// Text in the middle of a pane wraps at this width.
const TEXT_WIDTH: f32 = 360.0;

/// `child` in the middle of the room it is given.
pub fn centered(child: impl IntoView) -> Div {
    div()
        .flex_1()
        .size_full()
        .min_h(0)
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .p(MARGIN)
        .child(child)
}

/// What a pane shows while what it is for loads. `label` names the wait for assistive technology.
pub fn pane_loading(label: impl Into<String>) -> Div {
    let label = label.into();
    centered(loading_indicator().access(move |a| a.role(Role::ProgressBar).label(label.clone())))
}

/// What a pane shows when there is nothing in it: what is missing, why, if that helps, and what to
/// do about it, if there is something.
pub fn empty_state(message: &str, note: Option<&str>, action: Option<Div>) -> Div {
    centered(
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(4.0_f32)
            .max_w(TEXT_WIDTH)
            .text_align(TextAlign::Center)
            .child(muted_text(TextStyle::BODY_LARGE, message.to_owned()))
            .children(note.map(|note| muted_text(TextStyle::BODY_MEDIUM, note.to_owned())))
            .children(action.map(|action| action.mt(12))),
    )
}

/// What a list shows when its filter leaves nothing. `note` is what
/// `Session::missing_readings_note` gives.
pub fn no_matches(message: &str, note: Option<&str>) -> Div {
    empty_state(message, note, None)
}

/// What a pane shows when what it is for could not be loaded: `title` says what, `reason` why,
/// and Retry runs `retry`.
pub fn load_failure(
    title: impl Into<String>,
    reason: impl Into<String>,
    retry: impl Fn() + 'static,
) -> Div {
    let title = title.into();
    centered(
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(4.0_f32)
            .max_w(TEXT_WIDTH)
            .text_align(TextAlign::Center)
            .child(
                styled_text(TextStyle::TITLE_MEDIUM, title.clone())
                    .access(move |a| a.role(Role::Heading).level(2).label(title.clone())),
            )
            .child(error_text(reason.into()))
            .child(retry_button(retry).mt(12)),
    )
}

/// The button that tries again what failed.
pub fn retry_button(retry: impl Fn() + 'static) -> Div {
    styled_button("Retry", ButtonStyle::tonal()).on_click(move |_| retry())
}

#[cfg(test)]
mod tests;
