use subsonic::{AlbumId, SongId};

/// The narrowest an album card goes, in rem.
const CARD_MIN_WIDTH: f32 = 10.;
/// The space between album cards, in rem.
pub(crate) const CARD_GAP: f32 = 1.;
/// The space on either side of the screen's content, in rem.
pub(crate) const INSET: f32 = 1.5;
/// How many rows Page Up and Page Down move. Rows of songs and of albums differ
/// too much in height for a page to be a number of them; this stays in view.
const PAGE_ROWS: usize = 4;

/// How many album cards fit across a screen `width` rem wide.
pub(crate) fn columns_for(width: f32) -> usize {
    let room = width - INSET * 2. + CARD_GAP;
    ((room / (CARD_MIN_WIDTH + CARD_GAP)) as usize).max(1)
}

/// Something on the screen that can be selected, by identity, so that the
/// selection survives a reload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Item {
    Song(SongId),
    Album(AlbumId),
}

/// A keyboard move among the items.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Motion {
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
/// starts at the first item, or the last for [`Motion::Last`].
pub(crate) fn step(rows: &[Vec<Item>], current: Option<&Item>, motion: Motion) -> Option<Item> {
    let first = || rows.iter().flatten().next().cloned();
    let last = || rows.iter().flatten().next_back().cloned();
    let position = current.and_then(|current| {
        rows.iter().enumerate().find_map(|(row_ix, row)| {
            let column = row.iter().position(|item| item == current)?;
            Some((row_ix, column))
        })
    });
    let Some((row_ix, column)) = position else {
        return if motion == Motion::Last {
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
        Motion::Up => in_row(row_ix.saturating_sub(1), column),
        Motion::Down => in_row(row_ix + 1, column).or_else(|| in_row(row_ix, column)),
        Motion::Left => in_row(row_ix, column.saturating_sub(1)),
        Motion::Right => in_row(row_ix, column + 1),
        Motion::PageUp => in_row(row_ix.saturating_sub(PAGE_ROWS), column),
        Motion::PageDown => in_row((row_ix + PAGE_ROWS).min(rows.len() - 1), column),
        Motion::First => first(),
        Motion::Last => last(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(id: &str) -> Item {
        Item::Song(SongId::new(id))
    }

    fn album(id: &str) -> Item {
        Item::Album(AlbumId::new(id))
    }

    /// Two songs above two rows of albums, the second one short.
    fn rows() -> Vec<Vec<Item>> {
        vec![
            vec![song("s1")],
            vec![song("s2")],
            vec![album("a1"), album("a2"), album("a3")],
            vec![album("a4")],
        ]
    }

    #[test]
    fn a_move_with_nothing_selected_starts_at_an_end() {
        for motion in [Motion::Up, Motion::Down, Motion::Left, Motion::Right] {
            assert_eq!(step(&rows(), None, motion), Some(song("s1")));
        }
        assert_eq!(step(&rows(), None, Motion::Last), Some(album("a4")));
    }

    #[test]
    fn a_selection_no_longer_on_the_screen_starts_over() {
        assert_eq!(
            step(&rows(), Some(&album("gone")), Motion::Down),
            Some(song("s1"))
        );
    }

    #[test]
    fn nothing_to_select_stays_nothing() {
        assert_eq!(step(&[], None, Motion::Down), None);
        assert_eq!(step(&[], Some(&song("s1")), Motion::Last), None);
    }

    #[test]
    fn up_and_down_keep_the_column_where_the_row_has_it() {
        let rows = rows();
        assert_eq!(
            step(&rows, Some(&song("s2")), Motion::Down),
            Some(album("a1"))
        );
        assert_eq!(
            step(&rows, Some(&album("a1")), Motion::Down),
            Some(album("a4"))
        );
        assert_eq!(
            step(&rows, Some(&album("a3")), Motion::Down),
            Some(album("a4"))
        );
        assert_eq!(
            step(&rows, Some(&album("a3")), Motion::Up),
            Some(song("s2"))
        );
    }

    #[test]
    fn the_ends_hold() {
        let rows = rows();
        assert_eq!(step(&rows, Some(&song("s1")), Motion::Up), Some(song("s1")));
        assert_eq!(
            step(&rows, Some(&album("a4")), Motion::Down),
            Some(album("a4"))
        );
        assert_eq!(
            step(&rows, Some(&album("a1")), Motion::Left),
            Some(album("a1"))
        );
        assert_eq!(
            step(&rows, Some(&album("a3")), Motion::Right),
            Some(album("a3"))
        );
    }

    #[test]
    fn left_and_right_stay_in_the_row() {
        let rows = rows();
        assert_eq!(
            step(&rows, Some(&album("a1")), Motion::Right),
            Some(album("a2"))
        );
        assert_eq!(
            step(&rows, Some(&album("a2")), Motion::Left),
            Some(album("a1"))
        );
        assert_eq!(
            step(&rows, Some(&song("s1")), Motion::Right),
            Some(song("s1"))
        );
    }

    #[test]
    fn pages_move_several_rows_and_stop_at_the_ends() {
        let rows: Vec<_> = (0..10).map(|ix| vec![song(&format!("s{ix}"))]).collect();
        let page = |from: &str, motion| step(&rows, Some(&song(from)), motion);
        assert_eq!(page("s1", Motion::PageDown), Some(song("s5")));
        assert_eq!(page("s8", Motion::PageDown), Some(song("s9")));
        assert_eq!(page("s5", Motion::PageUp), Some(song("s1")));
        assert_eq!(page("s2", Motion::PageUp), Some(song("s0")));
    }

    #[test]
    fn first_and_last_reach_the_ends() {
        let rows = rows();
        assert_eq!(
            step(&rows, Some(&album("a2")), Motion::First),
            Some(song("s1"))
        );
        assert_eq!(
            step(&rows, Some(&song("s2")), Motion::Last),
            Some(album("a4"))
        );
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
