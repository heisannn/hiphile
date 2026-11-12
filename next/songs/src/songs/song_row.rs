use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::typography::TextStyle;
use hiphile_core::favorites::Favorite;
use next_ui::link::{REACH, album_link, song_artist_link};
use next_ui::row_buttons::RowButtons;
use next_ui::row_list::RowState;
use next_ui::row_text::faded_end;
use next_ui::song_row::{SongRow, song_words};
use subsonic::{Song, SongId};

use super::columns::words;
use super::table::Table;

/// What the row of `song` shows: its title, its artist and album as links, whether it plays, how
/// long it is, and its buttons. Its words fade while it would be skipped.
pub(super) fn song_row(table: &Table, song: &Song, row: RowState<SongId>) -> Div {
    let commands = &table.cx.commands;
    let known = table.cx.known;
    let state = {
        let (table, id) = (table.clone(), song.id().clone());
        move || table.state_of(&id)
    };
    let album = song
        .album()
        .map(|name| album_link(name, song.album_id().cloned(), commands));
    let words = words(
        song_words(song, None),
        link_cell(song_artist_link(song, commands)),
        link_cell(album),
    );
    let groups = {
        let (table, id) = (table.clone(), song.id().clone());
        move || table.menu_groups(table.songs_for(&id))
    };
    let buttons = RowButtons::new(
        Favorite::Song(song.clone()),
        commands,
        known,
        table.menu,
        groups,
    )
    .selected_when(move || row.is_selected());
    let skipped = song.clone();
    next_ui::song_row::song_row(
        SongRow::new(song, words, state)
            .skipped_when(move || known.is_skipped(&skipped))
            .buttons(buttons),
    )
}

/// A name in a lane, which goes where it names when clicked, cut and faded where it does not fit.
/// Its hover reaches past it into the gaps beside the lane.
fn link_cell(link: Option<Div>) -> Div {
    let line = TextStyle::BODY_MEDIUM
        .apply(div())
        .flex()
        .flex_row()
        .items_center()
        .px(REACH)
        .dyn_style(|s| s.text_color(color_scheme().on_surface_variant))
        .children(link);
    faded_end(line).mx(-REACH).flex_1()
}
