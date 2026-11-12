use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::typography::TextStyle;
use hiphile_core::favorites::Favorite;
use next_ui::context_menu::more_button;
use next_ui::details::format_duration;
use next_ui::favorite_button::favorite_button;
use next_ui::link::{REACH, album_link, song_artist_link};
use next_ui::now_playing::{play_state_icon, skippable};
use next_ui::row_list::RowState;
use next_ui::row_text::{faded_end, row_text};
use next_ui::type_scale::muted_text;
use subsonic::{Song, SongId};

use super::columns::{lanes, row_line};
use super::table::Table;

/// What the row of `song` shows: whether it plays, its title, its artist and album as links, how
/// long it is, and its favorite and More buttons. Its words fade while it would be skipped.
///
/// The buttons are shortcuts for the menu and the keys, so they show only while the row is under
/// the pointer, selected, or the one whose menu is open, apart from a favorite's filled heart.
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
    let time = song.duration().map(format_duration).unwrap_or_default();
    let words = lanes(
        play_state_icon(state),
        row_text(song.title().to_owned(), None).flex_1(),
        link_cell(song_artist_link(song, commands)),
        link_cell(album),
        muted_text(TextStyle::BODY_MEDIUM, time),
    );
    let skipped = {
        let song = song.clone();
        move || known.is_skipped(&song)
    };
    let (hovered, opened, menu) = (signal(false), signal(false), table.menu);
    effect(move || {
        if !menu.is_open() {
            untrack(|| opened.set(false));
        }
    });
    let shows =
        memo(move || hovered.get() || row.is_selected() || (opened.get() && menu.is_open()));
    let more = {
        let (table, id) = (table.clone(), song.id().clone());
        move || {
            opened.set(true);
            table.menu_groups(table.songs_for(&id))
        }
    };
    let favorite = Favorite::Song(song.clone());
    let target = favorite.target();
    let is_favorite = move || known.is_favorite(&target) == Some(true);
    let end = div()
        .flex()
        .flex_row()
        .items_center()
        .child(shown_when(
            favorite_button(favorite, commands, known),
            move || shows.get() || is_favorite(),
        ))
        .child(shown_when(more_button(table.menu, more), move || {
            shows.get()
        }));
    row_line(skippable(words, skipped), end).on_hover(move |on| hovered.set(on))
}

/// `control`, which stays in its place and can be reached while it is hidden.
fn shown_when(control: Div, shows: impl Fn() -> bool + 'static) -> Div {
    control.dyn_style(move |s| if shows() { s } else { s.opacity(0.0_f32) })
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
