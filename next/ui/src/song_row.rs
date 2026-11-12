//! The row of a song, the same wherever songs are listed: its number or its cover, its title and
//! what is under it, whether it plays, how long it lasts, and the buttons of the library.

use std::rc::Rc;
use std::time::Duration;

use anti_webui::prelude::*;
use anti_webui_md3e::typography::TextStyle;
use subsonic::Song;

use crate::details::format_duration;
use crate::now_playing::{PLAY_STATE_WIDTH, PlayState, play_state_icon, skippable};
use crate::pane::INSET;
use crate::row_buttons::{BUTTONS_LANE, LANE_GAP, RowButtons, row_line, row_with_buttons};
use crate::row_list::ROW_PADDING;
use crate::row_text::faded_end;
use crate::type_scale::muted_text;

/// The lane of a song's number, as wide as four figures.
pub const NUMBER_LANE: f32 = 32.0;
/// The lane of how long a song lasts, with room for `h:mm:ss`.
pub const DURATION_LANE: f32 = 56.0;

/// What [`song_row`] shows.
pub struct SongRow {
    words: Div,
    number: Option<String>,
    cover: Option<Div>,
    state: Rc<dyn Fn() -> Option<PlayState>>,
    duration: Option<Duration>,
    skipped: Option<Rc<dyn Fn() -> bool>>,
    buttons: Option<RowButtons>,
}

impl SongRow {
    /// The row of `song`, with `words` in the room the rest leaves, such as [`song_words`].
    /// `state` says whether it is the song that plays, and may read signals.
    pub fn new(song: &Song, words: Div, state: impl Fn() -> Option<PlayState> + 'static) -> Self {
        Self {
            words,
            number: None,
            cover: None,
            state: Rc::new(state),
            duration: song.duration(),
            skipped: None,
            buttons: None,
        }
    }

    /// Where the song is, such as its track on an album or its place in a playlist.
    pub fn number(mut self, number: Option<u32>) -> Self {
        self.number = number.map(|number| number.to_string());
        self
    }

    /// Before the words, such as a [`row_cover`](crate::row_cover::row_cover).
    pub fn cover(mut self, cover: Div) -> Self {
        self.cover = Some(cover);
        self
    }

    /// Whether the player would skip the song, which fades all but the buttons. It may read
    /// signals.
    pub fn skipped_when(mut self, skipped: impl Fn() -> bool + 'static) -> Self {
        self.skipped = Some(Rc::new(skipped));
        self
    }

    pub fn buttons(mut self, buttons: RowButtons) -> Self {
        self.buttons = Some(buttons);
        self
    }
}

/// The title of `song`, over `line` when there is one, such as who it is by and where it is from
/// ([`crate::link::song_line`]). What does not fit is cut and fades at its end.
pub fn song_words(song: &Song, line: Option<Div>) -> Div {
    div()
        .flex()
        .flex_col()
        .justify_center()
        .min_w(0)
        .child(faded_end(
            TextStyle::TITLE_SMALL
                .apply(div())
                .child(text(song.title().to_owned())),
        ))
        .children(line)
}

/// The number, the cover, the words, whether the song plays, how long it lasts and the buttons, in
/// lanes that are as wide in every row, so that each is in line with the same of the others.
pub fn song_row(row: SongRow) -> Div {
    let SongRow {
        words,
        number,
        cover,
        state,
        duration,
        skipped,
        buttons,
    } = row;
    let number = number.map(|number| {
        lane(NUMBER_LANE)
            .justify_end()
            .child(muted_text(TextStyle::BODY_MEDIUM, number))
    });
    let duration = muted_text(
        TextStyle::BODY_MEDIUM,
        duration.map(format_duration).unwrap_or_default(),
    );
    let content = lanes(
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(LANE_GAP)
            .children(number)
            .children(cover.map(|cover| cover.flex_none()))
            .child(words.flex_1()),
        play_state_icon(move || state()),
        duration,
    );
    let content = match skipped {
        Some(skipped) => skippable(content, move || skipped()),
        None => content,
    };
    row_with_buttons(content, buttons)
}

/// Headings over rows of songs with neither a number nor a cover, such as the columns of a table:
/// `words` over their words, and `duration` over how long they last.
pub fn song_headings(words: Div, duration: Div) -> Div {
    let content = lanes(words, div().w(PLAY_STATE_WIDTH), duration);
    let buttons = div().flex_none().w(BUTTONS_LANE);
    // As far from the edge as a row's frame is in its list
    div()
        .flex()
        .flex_row()
        .px(INSET - ROW_PADDING)
        .child(row_line(content, Some(buttons)))
}

/// `start`, which takes what room is left, then whether the song plays, and how long it lasts.
fn lanes(start: Div, state: Div, duration: Div) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(LANE_GAP)
        .min_w(0)
        .child(start.flex_1().min_w(0))
        .child(state.flex_none())
        .child(lane(DURATION_LANE).justify_end().child(duration))
}

/// A lane of `width`, whatever it holds.
fn lane(width: f32) -> Div {
    div().flex().flex_row().flex_none().items_center().w(width)
}

#[cfg(test)]
mod tests;
