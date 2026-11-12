use std::cell::RefCell;
use std::rc::Rc;

use anti_webui::prelude::*;
use hiphile_core::Shelf;
use hiphile_core::navigation::Destination;
use hiphile_core::songs::{Column, SongListStatus};
use next_ui::details::format_song_count;
use next_ui::list_pane::{ListHeader, list_filter, list_header, refresh_failure};
use next_ui::load_state::{SONGS, empty_state, load_failure, no_matches, pane_loading};
use next_ui::pane::pane;
use next_ui::sort_menu::{SortOption, sort_menu};
use next_ui::{Screen, ScreenContext};
use subsonic::{Song, SongId};

mod columns;
mod commands;
mod song_row;
mod table;

use table::Table;

/// Every song in the library, which the workspace reads once the screen first shows: a table whose
/// headings sort it, under a filter, with the order also chosen from the bar.
pub fn songs(cx: &ScreenContext) -> Screen {
    commands::bind_song_keys();
    let table = Table::new(cx);
    keep_first_selected(&table);
    let body = div()
        .flex()
        .flex_col()
        .child(filter(&table))
        .child(header(&table))
        .child(failed_refresh(&table))
        .child(content(&table))
        .child(table.menu.view());
    Screen::new(Destination::Songs.label(), pane(body))
        .action(sort(&table))
        .home(table.list)
}

/// What the pane shows below its header.
#[derive(Clone, Debug, PartialEq)]
enum Phase {
    Loading,
    /// The songs could not be read, for this reason.
    Failed(String),
    /// The library has none.
    Empty,
    /// The filter, as typed, leaves none.
    NoMatches(String),
    Rows,
}

impl Phase {
    fn of(status: &SongListStatus) -> Self {
        match (status.shown(), status.songs()) {
            (Some(shown), _) if shown.items().is_empty() => Self::Empty,
            (Some(shown), _) if shown.visible().is_empty() => {
                Self::NoMatches(status.query().trim().to_owned())
            }
            (Some(_), _) => Self::Rows,
            (None, Shelf::Failed(reason)) => Self::Failed(reason.clone()),
            (None, Shelf::Loading | Shelf::Loaded(_)) => Self::Loading,
        }
    }
}

fn content(table: &Table) -> Div {
    let status = table.status;
    let phase = memo(move || status.with(Phase::of));
    let table = table.clone();
    div()
        .flex_1()
        .min_h(0)
        .flex()
        .flex_col()
        .child(dyn_view(move || {
            let table = table.clone();
            match phase.get() {
                Phase::Loading => untrack(|| pane_loading(SONGS.loading()).into_view()),
                Phase::Failed(reason) => untrack(|| {
                    let song_list = table.song_list.clone();
                    load_failure(SONGS.load_failed(), reason, move || song_list.reload())
                        .into_view()
                }),
                Phase::Empty => {
                    untrack(|| empty_state(&SONGS.none_on_server(), None, None).into_view())
                }
                Phase::NoMatches(query) => untrack(|| {
                    let note = table.cx.workspace.session().missing_readings_note();
                    no_matches(move || query.clone(), note).into_view()
                }),
                Phase::Rows => untrack(|| table.view().into_view()),
            }
        }))
}

/// The filter, which starts from the one the songs are filtered by, as the list outlives the
/// screen. Enter goes on to the rows.
fn filter(table: &Table) -> Div {
    let song_list = table.song_list.clone();
    let value = signal(untrack(|| {
        table.status.with(|status| status.query().to_owned())
    }));
    effect(move || {
        let query = value.get();
        untrack(|| {
            if song_list.status().query() != query {
                song_list.set_query(&query);
            }
        });
    });
    list_filter(value, "Filter Songs", table.list)
}

fn header(table: &Table) -> Div {
    let status = table.status;
    let count = move || {
        status.with(|status| {
            let shown = status.shown()?;
            Some(format_song_count(shown.items().len()))
        })
    };
    let is_updating = move || status.with(|status| status.shown().is_some() && status.is_loading());
    list_header(ListHeader::new(count).updating(SONGS.updating(), is_updating))
}

/// That reading the songs again failed, while those read before stay on show.
fn failed_refresh(table: &Table) -> Div {
    let status = table.status;
    let failure = memo(move || status.with(|status| status.failure().map(str::to_owned)));
    let song_list = table.song_list.clone();
    div().flex().flex_col().flex_none().child(dyn_view(move || {
        let song_list = song_list.clone();
        match failure.get() {
            Some(reason) => untrack(|| {
                refresh_failure(SONGS.refresh_failed(), reason, move || song_list.reload())
                    .into_view()
            }),
            None => untrack(|| div().into_view()),
        }
    }))
}

/// The orders of the menu in the bar. The dates have no heading, and every order is in one group,
/// as the button names the order of its first.
fn sort(table: &Table) -> Div {
    let sort = table.sort;
    let by = |label: &str, column: Column| {
        let song_list = table.song_list.clone();
        SortOption::new(
            label,
            move || sort.get().is_some_and(|sort| sort.column == column),
            move || song_list.choose_sort(column),
        )
    };
    let way = |label: &str, is_descending: bool| {
        let song_list = table.song_list.clone();
        SortOption::new(
            label,
            move || {
                sort.get()
                    .is_some_and(|sort| sort.is_descending == is_descending)
            },
            move || song_list.choose_direction(is_descending),
        )
    };
    sort_menu(
        table.cx.overlays,
        move || sort.get().is_some_and(|sort| sort.is_descending),
        vec![
            vec![
                by("Title", Column::Title),
                by("Artist", Column::Artist),
                by("Album", Column::Album),
                by("Time", Column::Time),
                by("Recently Added", Column::Added),
                by("Release Date", Column::Released),
            ],
            vec![way("Ascending", false), way("Descending", true)],
        ],
    )
}

/// Until the user chooses a song, by pressing a row or with the keys, the first on show is the one
/// selected, which the keys act on. A new order or filter then selects the new first rather than
/// chase the old one down the list.
fn keep_first_selected(table: &Table) {
    let (shown, selection, chose) = (table.shown, table.selection, table.chose);
    let first = memo(move || {
        shown.with(|shown| {
            let song = shown.as_ref()?.get(0)?;
            Some(song.id().clone())
        })
    });
    let picked: Rc<RefCell<Option<SongId>>> = Rc::default();
    effect(move || {
        let first = first.get();
        untrack(|| {
            let is_chosen = chose.get()
                || selection.with(|selected| {
                    let picked = picked.borrow();
                    let on_show = shown.with(|shown| {
                        shown.as_ref().map_or(0, |shown| {
                            selected.count_in(
                                (0..shown.visible().len())
                                    .filter_map(|position| shown.get(position))
                                    .map(Song::id),
                            )
                        })
                    });
                    let is_picked =
                        on_show == 1 && picked.as_ref().is_some_and(|id| selected.contains(id));
                    on_show > 0 && !is_picked
                });
            if is_chosen {
                return;
            }
            match &first {
                Some(first) => selection.select(first.clone()),
                None => selection.clear(),
            }
            *picked.borrow_mut() = first;
        });
    });
}

#[cfg(test)]
mod tests;
