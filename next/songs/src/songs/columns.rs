use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::components::{Ink, sized_icon};
use anti_webui_md3e::icons;
use anti_webui_md3e::typography::TextStyle;
use hiphile_core::songs::Column;
use next_ui::link::REACH;
use next_ui::now_playing::PLAY_STATE_WIDTH;
use next_ui::pane::INSET;
use next_ui::row_list::ROW_PADDING;

use super::table::Table;

/// As high as the buttons at a row's end.
pub(super) const ROW_HEIGHT: f32 = 40.0;
/// Between the lanes of a row.
const GAP: f32 = 12.0;
/// Room for `h:mm:ss`.
const TIME_WIDTH: f32 = 56.0;
/// What a small icon button has between its edge and its icon, which brings the buttons at a
/// row's end closer to it than text for their icons to line up with the pane's.
const BUTTON_PADDING: f32 = 8.0;
/// The favorite and More buttons at a row's end.
pub(super) const END_WIDTH: f32 = 2.0 * 40.0;
const HEADINGS_HEIGHT: f32 = 32.0;
const ARROW: f32 = 16.0;

/// A row's lanes, which the headings share: the play state, the title, the artist, the album and
/// the time. The names share what the row has left, the title the most.
pub(super) fn lanes(state: Div, title: Div, artist: Div, album: Div, time: Div) -> Div {
    let lane = || div().flex().flex_row().items_center();
    let share = |part: f32| lane().flex_grow(part).flex_basis(0.0_f32).min_w(0);
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(GAP)
        .min_w(0)
        .child(lane().flex_none().w(PLAY_STATE_WIDTH).child(state))
        .child(share(3.0).child(title))
        .child(share(2.0).child(artist))
        .child(share(2.0).child(album))
        .child(lane().justify_end().flex_none().w(TIME_WIDTH).child(time))
}

/// The lanes and the buttons at the end of a row, which start on the line the pane's text does and
/// end where its buttons do.
pub(super) fn row_line(lanes: Div, end: Div) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(GAP)
        .size_full()
        .pl(ROW_PADDING)
        .pr(ROW_PADDING - BUTTON_PADDING)
        .child(lanes.flex_1())
        .child(end.flex_none().w(END_WIDTH))
}

/// The headings above the rows, each of which sorts the songs by its lane and turns them around
/// when they already go by it.
pub(super) fn headings(table: &Table) -> Div {
    let heading = |label: &'static str, column: Column| heading(table, label, column);
    div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .gap(GAP)
        .h(HEADINGS_HEIGHT)
        .pl(INSET)
        .pr(INSET - BUTTON_PADDING)
        .child(
            lanes(
                div(),
                heading("Title", Column::Title),
                heading("Artist", Column::Artist),
                heading("Album", Column::Album),
                heading("Time", Column::Time),
            )
            .flex_1(),
        )
        .child(div().flex_none().w(END_WIDTH))
}

/// A heading named `label`. Its arrow says which way the songs run while they go by `column`; it
/// comes before the label of the time, whose lane ends where its words do.
///
/// It is no stop of Tab: from the keyboard, the menu in the bar sorts.
fn heading(table: &Table, label: &'static str, column: Column) -> Div {
    let sort = table.sort;
    let song_list = table.song_list.clone();
    let way = memo(move || {
        sort.get()
            .filter(|sort| sort.column == column)
            .map(|sort| sort.is_descending)
    });
    let ink = Ink::new(|c| c.on_surface, || false);
    let hovered = ink.hovered;
    let arrow = show(
        move || way.get().is_some(),
        move || {
            sized_icon(icons::ARROW_UPWARD, ARROW).dyn_style(move |s| {
                s.rotate(if way.get() == Some(true) {
                    180.0_f32
                } else {
                    0.0
                })
            })
        },
    );
    let name = move || match way.get() {
        Some(false) => format!("{label}, Ascending"),
        Some(true) => format!("{label}, Descending"),
        None => label.to_owned(),
    };
    let words = div().flex_none().child(text(label));
    let button = TextStyle::LABEL_MEDIUM
        .apply(div())
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .gap(4.0_f32)
        .px(REACH)
        .mx(-REACH)
        .py(REACH / 2.0)
        .rounded(4.0_f32)
        .access(move |a| a.role(Role::Button).label(name()))
        .dyn_style(move |s| {
            let c = color_scheme();
            let is_strong = way.get().is_some() || hovered.get();
            s.text_color(if is_strong {
                c.on_surface
            } else {
                c.on_surface_variant
            })
        })
        .on_click(move |_| song_list.sort_by(column));
    let button = if column == Column::Time {
        button.child(arrow).child(words)
    } else {
        button.child(words).child(arrow)
    };
    ink.paint(ink.listen(button))
}
