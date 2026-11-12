//! The lists of each kind and their rows.

use std::rc::Rc;
use std::sync::Arc;

use anti_webui::prelude::*;
use anti_webui_md3e::typography::TextStyle;
use hiphile_core::favorites::{Favorite, Section};
use next_ui::cover::CoverKind;
use next_ui::details::format_album_count;
use next_ui::link::{album_line, song_line};
use next_ui::now_playing::song_row_label;
use next_ui::row_buttons::{LANE_GAP, RowButtons, row_with_buttons};
use next_ui::row_cover::row_cover;
use next_ui::row_list::{RowList, RowSource, RowState, row_list};
use next_ui::row_text::faded_end;
use next_ui::song_row::{SongRow, song_words};
use next_ui::type_scale::muted_text;
use subsonic::{Album, Artist, Song, StarTarget};

use super::View;

/// The favorites of one kind as the core keeps them.
struct Listed<T>(Arc<Vec<T>>);

impl<T: 'static> RowSource for Listed<T> {
    type Item = T;

    fn len(&self) -> usize {
        self.0.len()
    }

    fn get(&self, position: usize) -> Option<&T> {
        self.0.get(position)
    }
}

/// The list of `section`, whose items `of` takes from the favorites.
fn list<T: Clone + PartialEq + 'static>(
    view: &Rc<View>,
    section: Section,
    label: &str,
    of: fn(&hiphile_core::favorites::FavoriteLists) -> Arc<Vec<T>>,
    target: fn(&T) -> StarTarget,
    row: impl Fn(&Rc<View>, &T, RowState<StarTarget>) -> Div + 'static,
) -> RowList<Listed<T>, StarTarget> {
    let shown = view.shown;
    let items = memo(move || shown.with(|shown| of(shown.lists())));
    let (for_row, opening, menu) = (view.clone(), view.clone(), view.clone());
    RowList::new(
        label,
        move || Listed(items.get()),
        target,
        move |item, state| row(&for_row, item, state),
    )
    .selection(view.selections[section.ix()])
    .node_ref(view.list)
    .on_open(move |target| opening.open(&target))
    .on_context_menu(move |target, at| menu.menu.open_at(at, menu.menu_of(&target)))
}

pub(super) fn song_list(view: &Rc<View>) -> Div {
    let (now_playing, known) = (view.now_playing, view.cx.known);
    let list = list(
        view,
        Section::Songs,
        "Favorite songs",
        |lists| lists.songs.clone(),
        |song| StarTarget::Song(song.id().clone()),
        song_row,
    )
    .row_label(move |song| {
        let state = now_playing.with(|now| now.as_ref()?.state_of(song.id()));
        song_row_label(song.title(), state, known.is_skipped(song))
    });
    row_list(list)
}

pub(super) fn album_list(view: &Rc<View>) -> Div {
    row_list(
        list(
            view,
            Section::Albums,
            "Favorite albums",
            |lists| lists.albums.clone(),
            |album| StarTarget::Album(album.id().clone()),
            album_row,
        )
        .row_label(|album| album.name().to_owned()),
    )
}

pub(super) fn artist_list(view: &Rc<View>) -> Div {
    row_list(
        list(
            view,
            Section::Artists,
            "Favorite artists",
            |lists| lists.artists.clone(),
            |artist| StarTarget::Artist(artist.id().clone()),
            artist_row,
        )
        .row_label(|artist| artist.name().to_owned()),
    )
}

/// The buttons of a row of `favorite`: the favorite's, which takes it out, and More.
fn buttons(view: &Rc<View>, favorite: Favorite, row: RowState<StarTarget>) -> RowButtons {
    let target = favorite.target();
    let menu = view.clone();
    RowButtons::new(
        favorite,
        &view.cx.commands,
        view.cx.known,
        view.menu,
        move || menu.menu_of(&target),
    )
    .selected_when(move || row.is_selected())
}

/// The cover, the title, and the line below it if there is one.
fn words(cover: Div, title: &str, details: Option<Div>) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(LANE_GAP)
        .child(cover)
        .child(
            div()
                .flex()
                .flex_col()
                .justify_center()
                .flex_1()
                .min_w(0)
                .child(faded_end(
                    TextStyle::TITLE_SMALL
                        .apply(div())
                        .child(text(title.to_owned())),
                ))
                .children(details),
        )
}

fn song_row(view: &Rc<View>, song: &Song, row: RowState<StarTarget>) -> Div {
    let workspace = &view.cx.workspace;
    let (now_playing, known) = (view.now_playing, view.cx.known);
    let id = song.id().clone();
    let state = move || now_playing.with(|now| now.as_ref()?.state_of(&id));
    let skipped = song.clone();
    let words = song_words(song, Some(song_line(song, &view.cx.commands)));
    next_ui::song_row::song_row(
        SongRow::new(song, words, state)
            .cover(row_cover(workspace, song.cover_art(), CoverKind::Album))
            .skipped_when(move || known.is_skipped(&skipped))
            .buttons(buttons(view, Favorite::Song(song.clone()), row)),
    )
}

fn album_row(view: &Rc<View>, album: &Album, row: RowState<StarTarget>) -> Div {
    let content = words(
        row_cover(&view.cx.workspace, album.cover_art(), CoverKind::Album),
        album.name(),
        Some(album_line(album, &view.cx.commands)),
    );
    let buttons = buttons(view, Favorite::Album(album.clone()), row);
    row_with_buttons(content, Some(buttons))
}

fn artist_row(view: &Rc<View>, artist: &Artist, row: RowState<StarTarget>) -> Div {
    // Zero says nothing of an artist the server knows for their songs
    let albums = artist
        .album_count()
        .filter(|count| *count > 0)
        .map(|count| {
            faded_end(muted_text(
                TextStyle::BODY_MEDIUM,
                format_album_count(count as usize),
            ))
        });
    let content = words(
        row_cover(&view.cx.workspace, artist.cover_art(), CoverKind::Artist),
        artist.name(),
        albums,
    );
    let buttons = buttons(view, Favorite::Artist(artist.clone()), row);
    row_with_buttons(content, Some(buttons))
}
