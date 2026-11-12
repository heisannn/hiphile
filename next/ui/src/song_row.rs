//! What the row of a song shows where the songs come from many albums, such as Home's recently
//! played, an artist's top songs or a playlist: its cover, its title, who it is by and where it is
//! from, whether it plays, and how long it lasts.

use anti_webui::prelude::*;
use anti_webui_md3e::typography::TextStyle;
use subsonic::Song;

use crate::Commands;
use crate::details::format_duration;
use crate::link::song_line;
use crate::now_playing::{PlayState, play_state_icon};
use crate::row_list::ROW_PADDING;
use crate::row_text::faded_end;
use crate::type_scale::muted_text;

/// Between the parts of a row.
const GAP: f32 = 12.0;

/// What the row of `song` shows, with `cover` first, such as a
/// [`row_cover`](crate::row_cover::row_cover). Its artist and album go there when clicked, through
/// `commands`. `state` says whether it is the song that plays.
pub fn song_with_cover(
    song: &Song,
    cover: Div,
    state: impl Fn() -> Option<PlayState> + 'static,
    commands: &Commands,
) -> Div {
    let words = div()
        .flex()
        .flex_col()
        .justify_center()
        .flex_1()
        .min_w(0)
        .child(faded_end(
            TextStyle::TITLE_SMALL
                .apply(div())
                .child(text(song.title().to_owned())),
        ))
        .child(song_line(song, commands));
    let duration = song.duration().map(format_duration).unwrap_or_default();
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(GAP)
        .px(ROW_PADDING)
        .size_full()
        .child(cover)
        .child(words)
        .child(play_state_icon(state))
        .child(muted_text(TextStyle::BODY_MEDIUM, duration).flex_none())
}

#[cfg(test)]
mod tests;
