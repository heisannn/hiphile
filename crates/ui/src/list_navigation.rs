//! Keyboard movement within a single-selection list, shared by the lists of
//! every feature.

use gpui_kit::{App, KeyBinding, UniformListScrollHandle, actions, px};

actions!(
    list,
    [
        SelectNext,
        SelectPrevious,
        SelectFirst,
        SelectLast,
        SelectPageDown,
        SelectPageUp
    ]
);

/// Binds the arrow, Home, End, and page keys to the selection moves in the
/// list whose key context is `context`.
pub fn bind_list_navigation(context: &'static str, cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("down", SelectNext, Some(context)),
        KeyBinding::new("up", SelectPrevious, Some(context)),
        KeyBinding::new("home", SelectFirst, Some(context)),
        KeyBinding::new("end", SelectLast, Some(context)),
        KeyBinding::new("pagedown", SelectPageDown, Some(context)),
        KeyBinding::new("pageup", SelectPageUp, Some(context)),
    ]);
}

/// A keyboard move within a list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    By(isize),
    Pages(isize),
    First,
    Last,
}

impl Motion {
    /// The position to select in a list of `len` rows with `page` rows in
    /// view, starting from `current`. With nothing selected, a move starts
    /// at the first row, or the last for [`Motion::Last`].
    pub fn apply(self, current: Option<usize>, len: usize, page: usize) -> Option<usize> {
        let last = len.checked_sub(1)?;
        let Some(current) = current else {
            return Some(if matches!(self, Self::Last) { last } else { 0 });
        };
        Some(match self {
            Self::By(delta) => current.saturating_add_signed(delta).min(last),
            Self::Pages(pages) => current
                .saturating_add_signed(pages.saturating_mul(page as isize))
                .min(last),
            Self::First => 0,
            Self::Last => last,
        })
    }
}

/// Rows that fit in a list's viewport, at least one, from its last layout.
///
/// Reads the handle's public state, which the list writes while it lays
/// out; should that be under way, a page is one row rather than a panic.
pub fn page_rows(scroll: &UniformListScrollHandle) -> usize {
    let Ok(state) = scroll.0.try_borrow() else {
        return 1;
    };
    let viewport = state.base_handle.bounds().size.height;
    let row = state.last_item_size.map(|size| size.item.height);
    match row {
        Some(row) if row > px(0.) => ((viewport / row).floor() as usize).max(1),
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::Motion;

    #[test]
    fn motions_stay_within_the_list() {
        assert_eq!(Motion::By(1).apply(Some(9), 10, 4), Some(9));
        assert_eq!(Motion::By(-1).apply(Some(0), 10, 4), Some(0));
        assert_eq!(Motion::Pages(1).apply(Some(2), 10, 4), Some(6));
        assert_eq!(Motion::Pages(1).apply(Some(8), 10, 4), Some(9));
        assert_eq!(Motion::Pages(-1).apply(Some(2), 10, 4), Some(0));
        assert_eq!(Motion::First.apply(Some(5), 10, 4), Some(0));
        assert_eq!(Motion::Last.apply(Some(5), 10, 4), Some(9));
    }

    #[test]
    fn a_motion_from_no_selection_starts_at_an_end() {
        assert_eq!(Motion::By(1).apply(None, 10, 4), Some(0));
        assert_eq!(Motion::Pages(-1).apply(None, 10, 4), Some(0));
        assert_eq!(Motion::Last.apply(None, 10, 4), Some(9));
        assert_eq!(Motion::By(1).apply(None, 0, 4), None);
    }
}
