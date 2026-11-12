//! The list of the artists, on the pane at the start.

use std::rc::Rc;

use anti_webui::prelude::*;
use hiphile_core::artists::{ArtistListStatus, album_count_to_show};
use hiphile_core::navigation::Location;
use hiphile_core::{Shelf, Shown};
use next_ui::Command;
use next_ui::cover::CoverKind;
use next_ui::details::{format_album_count, format_artist_count};
use next_ui::keymap::{KeyContext, key_context};
use next_ui::list_pane::{ListHeader, list_filter, list_header, refresh_failure};
use next_ui::load_state::{ARTISTS, empty_state, load_failure, no_matches, pane_loading};
use next_ui::row_cover::row_cover;
use next_ui::row_list::{ROW_PADDING, RowList, RowSource, is_context_click, row_list};
use next_ui::row_text::row_text;
use subsonic::Artist;

use super::{ARTISTS_CONTEXT, View};

/// Between the picture and the words.
const GAP: f32 = 12.0;

pub(super) fn list_pane(view: &Rc<View>) -> Div {
    let context = {
        let (enabled, run) = (view.clone(), view.clone());
        KeyContext::new(
            ARTISTS_CONTEXT,
            move |command| enabled.is_enabled_on_list(*command),
            move |command| run.run_on_list(command),
        )
    };
    div()
        .flex()
        .flex_col()
        .child(list_filter(view.filter, "Filter Artists", view.list_node))
        .child(header(view))
        .child(refresh_error(view))
        .child(key_context(body(view.clone()), &context))
}

/// How many artists there are, and whether they are being read again.
fn header(view: &Rc<View>) -> Div {
    let status = view.status;
    let count = memo(move || status.with(|status| Some(status.shown()?.items().len())));
    list_header(
        ListHeader::new(move || count.get().map(format_artist_count))
            .updating(ARTISTS.updating(), move || {
                count.get().is_some() && status.with(ArtistListStatus::is_refreshing)
            }),
    )
}

/// Why reading the artists again failed, above those still on show.
fn refresh_error(view: &Rc<View>) -> Div {
    let status = view.status;
    let error = memo(move || status.with(|status| status.failure().map(str::to_owned)));
    let list = view.list.clone();
    div().flex().flex_col().flex_none().child(dyn_view(move || {
        let list = list.clone();
        match error.get() {
            None => untrack(|| div().into_view()),
            Some(reason) => untrack(|| {
                refresh_failure(ARTISTS.refresh_failed(), reason, move || list.reload()).into_view()
            }),
        }
    }))
}

/// What the pane shows below its header.
#[derive(Clone, Debug, PartialEq)]
enum Shape {
    Loading,
    Failed(String),
    Empty,
    NoMatches,
    Listed,
}

fn body(view: Rc<View>) -> Div {
    let status = view.status;
    let shape = memo(move || {
        status.with(|status| match (status.shown(), status.artists()) {
            (Some(shown), _) if shown.items().is_empty() => Shape::Empty,
            (Some(shown), _) if shown.visible().is_empty() => Shape::NoMatches,
            (Some(_), _) => Shape::Listed,
            (None, Shelf::Failed(error)) => Shape::Failed(error.clone()),
            (None, Shelf::Loading | Shelf::Loaded(_)) => Shape::Loading,
        })
    });
    div()
        .flex_1()
        .min_h(0)
        .flex()
        .flex_col()
        .child(dyn_view(move || {
            let shape = shape.get();
            let view = view.clone();
            untrack(move || shaped(&view, shape).flex_1().min_h(0).into_view())
        }))
}

fn shaped(view: &Rc<View>, shape: Shape) -> Div {
    match shape {
        Shape::Loading => pane_loading(ARTISTS.loading()),
        Shape::Failed(reason) => {
            let retrying = view.clone();
            load_failure(ARTISTS.load_failed(), reason, move || {
                retrying.focus_list();
                retrying.list.reload();
            })
        }
        Shape::Empty => empty_state(&ARTISTS.none_on_server(), None, None),
        Shape::NoMatches => {
            let (filter, note) = (
                view.filter,
                view.cx.workspace.session().missing_readings_note(),
            );
            no_matches(move || filter.get(), note)
        }
        Shape::Listed => artist_list(view),
    }
}

/// The artists the list shows, in their order.
struct Listed(Option<Shown<Artist>>);

impl RowSource for Listed {
    type Item = Artist;

    fn len(&self) -> usize {
        self.0.as_ref().map_or(0, |shown| shown.visible().len())
    }

    fn get(&self, position: usize) -> Option<&Artist> {
        self.0.as_ref()?.get(position)
    }
}

fn artist_list(view: &Rc<View>) -> Div {
    let status = view.status;
    let shown = memo(move || status.with(|status| status.shown().cloned()));
    let (for_row, opening, menu, laid_out) =
        (view.clone(), view.clone(), view.clone(), view.clone());
    let list = RowList::new(
        "Artists",
        move || Listed(shown.get()),
        |artist: &Artist| artist.id().clone(),
        move |artist, _| artist_row(&for_row, artist),
    )
    .selection(view.selection)
    .single_selection()
    .row_label(|artist| artist.name().to_owned())
    .node_ref(view.list_node)
    .on_open(move |_| opening.open_page())
    .on_context_menu(move |artist, at| {
        let groups = menu.artist_menu(&artist);
        menu.menu.open_at(at, groups);
    });
    row_list(list).on_resized(move |_, _| laid_out.list_laid_out())
}

/// The artist's picture, name and how many albums they have. Clicking it goes to them, which
/// going back returns from.
fn artist_row(view: &Rc<View>, artist: &Artist) -> Div {
    let details = album_count_to_show(artist).map(|count| format_album_count(count as usize));
    let to = Location::Artists(Some(artist.id().clone()));
    let commands = view.cx.commands.clone();
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(GAP)
        .size_full()
        .px(ROW_PADDING)
        .on_click(move |e| {
            if !is_context_click(e) {
                commands.send(Command::Show(to.clone()));
            }
        })
        .child(row_cover(
            &view.cx.workspace,
            artist.cover_art(),
            CoverKind::Artist,
        ))
        .child(row_text(artist.name().to_owned(), details).flex_1())
}
