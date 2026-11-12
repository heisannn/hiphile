//! The buttons at the end of a row of the library: the favorite's, and More, which opens the row's
//! menu.

use std::rc::Rc;

use anti_webui::prelude::*;
use anti_webui_md3e::components::MenuItem;
use hiphile_core::favorites::Favorite;

use crate::context_menu::{ContextMenu, more_button};
use crate::favorite_button::favorite_button;
use crate::row_list::ROW_PADDING;
use crate::{Commands, Known};

/// Between the lanes of a row.
pub const LANE_GAP: f32 = 12.0;
/// What a small icon button has between its edge and its icon, which brings the last of a row
/// closer to the edge than words for its icon to line up with theirs.
const BUTTON_PADDING: f32 = 8.0;
/// The lane of the favorite's button and More.
pub(crate) const BUTTONS_LANE: f32 = 2.0 * 40.0;

/// What [`row_with_buttons`] puts at the end of a row.
pub struct RowButtons {
    favorite: Favorite,
    commands: Commands,
    known: Known,
    menu: ContextMenu,
    groups: Rc<dyn Fn() -> Vec<Vec<MenuItem>>>,
    selected: Rc<dyn Fn() -> bool>,
}

impl RowButtons {
    /// The button that toggles `favorite`, through `commands`, and More, which opens `menu` with
    /// what `groups` makes then.
    pub fn new(
        favorite: Favorite,
        commands: &Commands,
        known: Known,
        menu: ContextMenu,
        groups: impl Fn() -> Vec<Vec<MenuItem>> + 'static,
    ) -> Self {
        Self {
            favorite,
            commands: commands.clone(),
            known,
            menu,
            groups: Rc::new(groups),
            selected: Rc::new(|| false),
        }
    }

    /// Whether the row is selected, which shows the buttons. It may read signals.
    pub fn selected_when(mut self, selected: impl Fn() -> bool + 'static) -> Self {
        self.selected = Rc::new(selected);
        self
    }
}

/// `content`, and `buttons` at the end, in a row that starts on the line the pane's text does and
/// ends where its buttons do, or its text without them.
///
/// The buttons are shortcuts for the menu and the keys, so they show only while the row is under
/// the pointer, selected, or the one whose menu is open, apart from a favorite's filled heart,
/// which says that it is one. Hidden, they keep their room, so that nothing before them moves.
pub fn row_with_buttons(content: Div, buttons: Option<RowButtons>) -> Div {
    let Some(buttons) = buttons else {
        return row_line(content, None);
    };
    let RowButtons {
        favorite,
        commands,
        known,
        menu,
        groups,
        selected,
    } = buttons;
    let (hovered, opened) = (signal(false), signal(false));
    effect(move || {
        if !menu.is_open() {
            untrack(|| opened.set(false));
        }
    });
    let shows = memo(move || hovered.get() || selected() || (opened.get() && menu.is_open()));
    let target = favorite.target();
    let is_favorite = move || known.is_favorite(&target) == Some(true);
    let more = more_button(menu, move || {
        opened.set(true);
        groups()
    });
    let end = div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .w(BUTTONS_LANE)
        .child(shown_when(
            favorite_button(favorite, &commands, known),
            move || shows.get() || is_favorite(),
        ))
        .child(shown_when(more, move || shows.get()));
    row_line(content, Some(end)).on_hover(move |on| hovered.set(on))
}

/// `content`, then `end` if there is one, such as the buttons, in a row as high as it is given.
pub(crate) fn row_line(content: Div, end: Option<Div>) -> Div {
    let line = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(LANE_GAP)
        .size_full()
        .pl(ROW_PADDING)
        .child(content.flex_1().min_w(0));
    match end {
        Some(end) => line.pr(ROW_PADDING - BUTTON_PADDING).child(end),
        None => line.pr(ROW_PADDING),
    }
}

/// `control`, which stays in its place and can be reached while it is hidden.
fn shown_when(control: Div, shows: impl Fn() -> bool + 'static) -> Div {
    control.dyn_style(move |s| if shows() { s } else { s.opacity(0.0_f32) })
}

#[cfg(test)]
mod tests;
