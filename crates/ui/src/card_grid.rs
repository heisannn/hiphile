//! Album and artist cards, and moving among rows of them.

use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::{
    App, Div, ElementId, InteractiveElement as _, IntoElement, ParentElement as _, Role,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, div,
    prelude::FluentBuilder as _,
};

use crate::CoverArt;

/// The narrowest a card goes, in rem.
const CARD_MIN_WIDTH: f32 = 10.;
/// The space between cards, in rem.
const CARD_GAP: f32 = 1.;
/// The space on either side of the cards, in rem.
const INSET: f32 = 1.5;
/// What a row of cards adds to its covers, in rem: their text and the space below.
const CARD_TEXT_HEIGHT: f32 = 4.125;
/// How many rows Page Up and Page Down move. Rows of songs and of cards differ
/// too much in height for a page to be a number of them; this stays in view.
const PAGE_ROWS: usize = 4;
/// The group of a card, for the hover outline over its cover.
const CARD_GROUP: &str = "media-card";

/// How many cards fit across a screen `width` rem wide.
pub fn columns_for(width: f32) -> usize {
    let room = width - INSET * 2. + CARD_GAP;
    ((room / (CARD_MIN_WIDTH + CARD_GAP)) as usize).max(1)
}

/// How tall a row of `columns` cards is across a screen `width` rem wide, in rem,
/// for reckoning the height of a list before its rows are laid out.
pub fn card_row_height(width: f32, columns: usize) -> f32 {
    let columns = columns.max(1) as f32;
    let cover = (width - INSET * 2. - CARD_GAP * (columns - 1.)) / columns;
    cover.max(0.) + CARD_TEXT_HEIGHT
}

/// A keyboard move among rows of items.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridMotion {
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    First,
    Last,
}

/// The item to select after `motion` from `current`, in `rows` as they show
/// from top to bottom. Without a current item that is on the screen, a move
/// starts at the first item, or the last for [`GridMotion::Last`].
pub fn step<T: Clone + PartialEq>(
    rows: &[Vec<T>],
    current: Option<&T>,
    motion: GridMotion,
) -> Option<T> {
    let first = || rows.iter().flatten().next().cloned();
    let last = || rows.iter().flatten().next_back().cloned();
    let position = current.and_then(|current| {
        rows.iter().enumerate().find_map(|(row_ix, row)| {
            let column = row.iter().position(|item| item == current)?;
            Some((row_ix, column))
        })
    });
    let Some((row_ix, column)) = position else {
        return if motion == GridMotion::Last {
            last()
        } else {
            first()
        };
    };
    let in_row = |row_ix: usize, column: usize| {
        let row = rows.get(row_ix)?;
        row.get(column.min(row.len().checked_sub(1)?)).cloned()
    };
    match motion {
        GridMotion::Up => in_row(row_ix.saturating_sub(1), column),
        GridMotion::Down => in_row(row_ix + 1, column).or_else(|| in_row(row_ix, column)),
        GridMotion::Left => in_row(row_ix, column.saturating_sub(1)),
        GridMotion::Right => in_row(row_ix, column + 1),
        GridMotion::PageUp => in_row(row_ix.saturating_sub(PAGE_ROWS), column),
        GridMotion::PageDown => in_row((row_ix + PAGE_ROWS).min(rows.len() - 1), column),
        GridMotion::First => first(),
        GridMotion::Last => last(),
    }
}

/// A row of cards, which take equal shares of its width.
pub fn card_row(columns: usize, cards: Vec<gpui_kit::AnyElement>) -> Div {
    let columns = u16::try_from(columns.max(1)).unwrap_or(u16::MAX);
    div()
        .flex_none()
        .px_6()
        .pb_4()
        .grid()
        .grid_cols(columns)
        .gap_4()
        .children(cards)
}

/// A cover with what it is of below it. The caller says what its clicks do.
pub fn media_card(
    id: impl Into<ElementId>,
    cover: CoverArt,
    title: SharedString,
    details: impl IntoElement,
    is_selected: bool,
    is_focused: bool,
    cx: &App,
) -> Stateful<Div> {
    // Selected, with or without focus, reads apart from hovered.
    let outline = if is_selected && is_focused {
        cx.theme().primary
    } else if is_selected {
        cx.theme().muted_foreground
    } else {
        cx.theme().transparent
    };
    let hover_outline = if is_selected {
        outline
    } else {
        cx.theme().border
    };
    v_flex()
        .id(id)
        .group(CARD_GROUP)
        .min_w_0()
        .gap_1()
        .role(Role::ListItem)
        .aria_label(title.clone())
        .aria_selected(is_selected)
        .when(is_selected, |this| this.aria_active_descendant())
        .child(
            div()
                .relative()
                .w_full()
                .child(cover.fill())
                // Over the cover, so that showing it moves nothing. Square
                // like the cover, whose corners a rounded frame does not clip.
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .border_2()
                        .border_color(outline)
                        .group_hover(CARD_GROUP, |this| this.border_color(hover_outline)),
                ),
        )
        .child(div().text_sm().truncate().child(title))
        .child(details)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two songs above two rows of cards, the second one short.
    fn rows() -> Vec<Vec<&'static str>> {
        vec![vec!["s1"], vec!["s2"], vec!["a1", "a2", "a3"], vec!["a4"]]
    }

    #[test]
    fn a_move_with_nothing_selected_starts_at_an_end() {
        for motion in [
            GridMotion::Up,
            GridMotion::Down,
            GridMotion::Left,
            GridMotion::Right,
        ] {
            assert_eq!(step(&rows(), None, motion), Some("s1"));
        }
        assert_eq!(step(&rows(), None, GridMotion::Last), Some("a4"));
    }

    #[test]
    fn a_selection_no_longer_on_the_screen_starts_over() {
        assert_eq!(step(&rows(), Some(&"gone"), GridMotion::Down), Some("s1"));
    }

    #[test]
    fn nothing_to_select_stays_nothing() {
        assert_eq!(step::<&str>(&[], None, GridMotion::Down), None);
        assert_eq!(step(&[], Some(&"s1"), GridMotion::Last), None);
    }

    #[test]
    fn up_and_down_keep_the_column_where_the_row_has_it() {
        let rows = rows();
        assert_eq!(step(&rows, Some(&"s2"), GridMotion::Down), Some("a1"));
        assert_eq!(step(&rows, Some(&"a1"), GridMotion::Down), Some("a4"));
        assert_eq!(step(&rows, Some(&"a3"), GridMotion::Down), Some("a4"));
        assert_eq!(step(&rows, Some(&"a3"), GridMotion::Up), Some("s2"));
    }

    #[test]
    fn the_ends_hold() {
        let rows = rows();
        assert_eq!(step(&rows, Some(&"s1"), GridMotion::Up), Some("s1"));
        assert_eq!(step(&rows, Some(&"a4"), GridMotion::Down), Some("a4"));
        assert_eq!(step(&rows, Some(&"a1"), GridMotion::Left), Some("a1"));
        assert_eq!(step(&rows, Some(&"a3"), GridMotion::Right), Some("a3"));
    }

    #[test]
    fn left_and_right_stay_in_the_row() {
        let rows = rows();
        assert_eq!(step(&rows, Some(&"a1"), GridMotion::Right), Some("a2"));
        assert_eq!(step(&rows, Some(&"a2"), GridMotion::Left), Some("a1"));
        assert_eq!(step(&rows, Some(&"s1"), GridMotion::Right), Some("s1"));
    }

    #[test]
    fn pages_move_several_rows_and_stop_at_the_ends() {
        let rows: Vec<_> = (0..10).map(|ix| vec![ix]).collect();
        let page = |from: i32, motion| step(&rows, Some(&from), motion);
        assert_eq!(page(1, GridMotion::PageDown), Some(5));
        assert_eq!(page(8, GridMotion::PageDown), Some(9));
        assert_eq!(page(5, GridMotion::PageUp), Some(1));
        assert_eq!(page(2, GridMotion::PageUp), Some(0));
    }

    #[test]
    fn first_and_last_reach_the_ends() {
        let rows = rows();
        assert_eq!(step(&rows, Some(&"a2"), GridMotion::First), Some("s1"));
        assert_eq!(step(&rows, Some(&"s2"), GridMotion::Last), Some("a4"));
    }

    #[test]
    fn columns_follow_the_width_and_never_reach_zero() {
        assert_eq!(columns_for(0.), 1);
        assert_eq!(columns_for(10.), 1);
        // 4 cards and 3 gaps need 43 rem, plus 3 of inset.
        assert_eq!(columns_for(45.9), 3);
        assert_eq!(columns_for(46.), 4);
        assert_eq!(columns_for(55.), 4);
        assert_eq!(columns_for(57.), 5);
    }
}
