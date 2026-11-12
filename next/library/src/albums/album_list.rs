use anti_webui::prelude::*;
use hiphile_core::library::AlbumListStatus;
use hiphile_core::{Shelf, Shown};
use next_ui::Command;
use next_ui::cover::CoverKind;
use next_ui::details::format_album_count;
use next_ui::keymap::{KeyContext, key_context};
use next_ui::link::album_line;
use next_ui::list_pane::{ListHeader, list_filter, list_header, refresh_failure};
use next_ui::load_state::{self, LIBRARY, empty_state, load_failure, no_matches, pane_loading};
use next_ui::menus::album_menu;
use next_ui::row_cover::row_cover;
use next_ui::row_list::{ROW_PADDING, RowList, RowSource, is_context_click, row_list};
use next_ui::row_text::row_text;
use subsonic::{Album, AlbumId};

use super::Model;
use super::keys::ALBUMS;

/// Between a row's cover and its words.
const COVER_GAP: f32 = 12.0;

/// What the album list shows in place of its rows.
#[derive(Clone, Debug, PartialEq)]
enum Standing {
    Loading,
    Failed(String),
    NoAlbums,
    NoMatches,
    Rows,
}

impl Standing {
    fn of(status: &AlbumListStatus) -> Self {
        match (status.albums(), status.shown()) {
            (Shelf::Failed(error), _) => Self::Failed(error.clone()),
            (Shelf::Loading, _) | (Shelf::Loaded(_), None) => Self::Loading,
            (Shelf::Loaded(_), Some(shown)) if shown.items().is_empty() => Self::NoAlbums,
            (Shelf::Loaded(_), Some(shown)) if shown.visible().is_empty() => Self::NoMatches,
            (Shelf::Loaded(_), Some(_)) => Self::Rows,
        }
    }
}

/// The albums on show, none until the first are.
struct Rows(Option<Shown<Album>>);

impl RowSource for Rows {
    type Item = Album;

    fn len(&self) -> usize {
        self.0.as_ref().map_or(0, |shown| shown.visible().len())
    }

    fn get(&self, position: usize) -> Option<&Album> {
        self.0.as_ref()?.get(position)
    }
}

/// The filter, how many albums there are, a refresh that failed, and the albums.
pub(super) fn album_list(model: &Model) -> Div {
    div()
        .flex()
        .flex_col()
        .child(list_filter(model.filter, "Filter Albums", model.album_rows))
        .child(header(model))
        .child(failure(model))
        .child(rows(model))
}

fn header(model: &Model) -> Div {
    let status = model.status;
    let count = memo(move || status.with(|status| status.shown().map(|shown| shown.items().len())));
    // Until there are albums, the list itself says it waits
    let is_updating =
        memo(move || count.get().is_some() && status.with(AlbumListStatus::is_updating));
    list_header(
        ListHeader::new(move || count.get().map(format_album_count))
            .updating(LIBRARY.updating(), move || is_updating.get()),
    )
}

fn failure(model: &Model) -> Div {
    let status = model.status;
    let failure = memo(move || status.with(|status| status.failure().map(str::to_owned)));
    let commands = model.cx.commands.clone();
    div().flex().flex_col().flex_none().child(dyn_view(move || {
        let commands = commands.clone();
        match failure.get() {
            Some(reason) => untrack(|| {
                refresh_failure(LIBRARY.refresh_failed(), reason, move || {
                    commands.send(Command::Refresh);
                })
                .into_view()
            }),
            None => untrack(|| div().into_view()),
        }
    }))
}

/// The rows, there in every state so that the focus and the keys stay with them, under what
/// stands in their place while there are none.
fn rows(model: &Model) -> Div {
    let status = model.status;
    let standing = memo(move || status.with(Standing::of));
    let context = {
        let (model, asked) = (model.clone(), model.clone());
        KeyContext::new(
            ALBUMS,
            move |key| asked.is_enabled_on_albums(*key),
            move |key| model.take_album_key(key),
        )
    };
    let laid_out = model.clone();
    let list = key_context(
        div()
            .flex()
            .flex_col()
            .size_full()
            .on_resized(move |_, _| laid_out.list_laid_out())
            .child(row_list(album_rows(model)).flex_1().min_h(0)),
        &context,
    );
    let in_place = model.clone();
    div()
        .relative()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(0)
        .child(list)
        .child(show(
            move || standing.get() != Standing::Rows,
            move || {
                let model = in_place.clone();
                div()
                    .absolute()
                    .inset(0)
                    .flex()
                    .flex_col()
                    .child(dyn_view(move || {
                        let model = model.clone();
                        let standing = standing.get();
                        untrack(|| in_place_of_rows(&model, standing).into_view())
                    }))
            },
        ))
}

fn in_place_of_rows(model: &Model, standing: Standing) -> Div {
    match standing {
        Standing::Loading => pane_loading(load_state::ALBUMS.loading()),
        Standing::Failed(reason) => {
            let (rows, commands) = (model.album_rows, model.cx.commands.clone());
            load_failure(load_state::ALBUMS.load_failed(), reason, move || {
                rows.focus();
                commands.send(Command::Refresh);
            })
        }
        Standing::NoAlbums => empty_state(&load_state::ALBUMS.none_on_server(), None, None),
        Standing::NoMatches => {
            let filter = model.filter;
            no_matches(
                move || filter.get(),
                model.cx.workspace.session().missing_readings_note(),
            )
        }
        Standing::Rows => div(),
    }
}

fn album_rows(model: &Model) -> RowList<Rows, AlbumId> {
    let status = model.status;
    let look = model.clone();
    let menu = model.clone();
    RowList::new(
        "Albums",
        move || Rows(status.with(|status| status.shown().cloned())),
        |album: &Album| album.id().clone(),
        move |album, _| album_row(&look, album),
    )
    .row_label(|album| album.name().to_owned())
    .selection(model.albums)
    .node_ref(model.album_rows)
    .on_context_menu(move |id, at| {
        let album = menu.status.with_untracked(|status| {
            status
                .albums()
                .items()
                .iter()
                .find(|album| album.id() == &id)
                .cloned()
        });
        if let Some(album) = album {
            let groups = album_menu(&menu.cx.commands, menu.cx.known, &album);
            menu.menu.open_at(at, groups);
        }
    })
}

/// The cover, the name, and the artist and the year. Choosing it with the pointer leaves the
/// album on show to go back to.
fn album_row(model: &Model, album: &Album) -> Div {
    let id = album.id().clone();
    let chooser = model.clone();
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(COVER_GAP)
        .px(ROW_PADDING)
        .size_full()
        .child(row_cover(
            &model.cx.workspace,
            album.cover_art(),
            CoverKind::Album,
        ))
        .child(
            row_text(album.name().to_owned(), None)
                .flex_1()
                .child(album_line(album, &model.cx.commands)),
        )
        .on_click(move |e| {
            if !is_context_click(e) {
                chooser.choose(id.clone());
            }
        })
}
