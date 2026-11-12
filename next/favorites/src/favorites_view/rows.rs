//! The lists of each kind and their rows.

use std::rc::Rc;
use std::sync::Arc;

use anti_webui::prelude::*;
use anti_webui_md3e::typography::TextStyle;
use hiphile_core::favorites::{Favorite, Section};
use next_ui::context_menu::more_button;
use next_ui::cover::CoverKind;
use next_ui::details::{format_album_count, format_duration};
use next_ui::favorite_button::favorite_button;
use next_ui::link::{album_line, song_line};
use next_ui::now_playing::{play_state_icon, skippable, song_row_label};
use next_ui::row_cover::row_cover;
use next_ui::row_list::{ROW_PADDING, RowList, RowSource, row_list};
use next_ui::row_text::faded_end;
use next_ui::type_scale::muted_text;
use subsonic::{Album, Artist, Song, StarTarget};

use super::View;

/// Between the cover, the words and what follows them.
const GAP: f32 = 12.0;

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
    row: impl Fn(&Rc<View>, &T) -> Div + 'static,
) -> RowList<Listed<T>, StarTarget> {
    let shown = view.shown;
    let items = memo(move || shown.with(|shown| of(shown.lists())));
    let (for_row, opening, menu) = (view.clone(), view.clone(), view.clone());
    RowList::new(
        label,
        move || Listed(items.get()),
        target,
        move |item, _| row(&for_row, item),
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

/// A row: `content`, then the favorite's button, which takes it out, and its menu.
fn row(view: &Rc<View>, content: Div, favorite: Favorite) -> Div {
    let target = favorite.target();
    let menu = view.clone();
    div()
        .flex()
        .flex_row()
        .items_center()
        .size_full()
        .pl(ROW_PADDING)
        .child(content.flex_1().min_w(0).h_full())
        .child(favorite_button(favorite, &view.cx.commands, view.cx.known))
        .child(more_button(view.menu, move || menu.menu_of(&target)))
}

/// The cover, the title, and the line below it if there is one.
fn words(cover: Div, title: &str, details: Option<Div>) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(GAP)
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

fn song_row(view: &Rc<View>, song: &Song) -> Div {
    let workspace = &view.cx.workspace;
    let (now_playing, known) = (view.now_playing, view.cx.known);
    let id = song.id().clone();
    let state = move || now_playing.with(|now| now.as_ref()?.state_of(&id));
    let duration = song.duration().map(format_duration).unwrap_or_default();
    let content = words(
        row_cover(workspace, song.cover_art(), CoverKind::Album),
        song.title(),
        Some(song_line(song, &view.cx.commands)),
    )
    .child(play_state_icon(state))
    .child(muted_text(TextStyle::BODY_MEDIUM, duration).flex_none());
    let skipped = {
        let song = song.clone();
        move || known.is_skipped(&song)
    };
    row(
        view,
        skippable(content, skipped),
        Favorite::Song(song.clone()),
    )
}

fn album_row(view: &Rc<View>, album: &Album) -> Div {
    let content = words(
        row_cover(&view.cx.workspace, album.cover_art(), CoverKind::Album),
        album.name(),
        Some(album_line(album, &view.cx.commands)),
    );
    row(view, content, Favorite::Album(album.clone()))
}

fn artist_row(view: &Rc<View>, artist: &Artist) -> Div {
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
    row(view, content, Favorite::Artist(artist.clone()))
}
