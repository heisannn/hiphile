use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{App, Div, IntoElement, ParentElement as _, SharedString, Styled as _, div};
use subsonic::Song;

use crate::PlayState;

pub const SKIPPED_OPACITY: f32 = 0.4;

/// `content` is what a row holds, not the row itself, whose selection and hover
/// keep their strength.
pub fn skippable_song(song: &Song, content: impl IntoElement, cx: &App) -> Div {
    div()
        .w_full()
        .min_w_0()
        .when(actions::is_skipped(song, cx), |this| {
            this.opacity(SKIPPED_OPACITY)
        })
        .child(content)
}

/// A screen reader's name for the row of `song`, which says what the row shows.
pub fn song_row_label(
    song: &Song,
    title: &SharedString,
    state: Option<PlayState>,
    cx: &App,
) -> SharedString {
    let label = PlayState::row_label(title, state);
    if actions::is_skipped(song, cx) {
        format!("{label}, skipped").into()
    } else {
        label
    }
}
