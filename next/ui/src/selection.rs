//! How a selected item of a list or a grid looks: filled, more strongly while the keys go to its
//! list. Its words keep the colors they have unselected: md3e's `on_secondary_container` is weaker
//! than the `on_surface_variant` of the details, and a title in it would read below them.

use std::rc::Rc;

use anti_webui::prelude::*;
use anti_webui_md3e::ColorScheme;

/// How much of the focused selection's fill an unfocused selection has, over the fill of a card at
/// rest. Less, and in a dark scheme it looks as a row the pointer is over does.
const UNFOCUSED: f32 = 0.5;

/// The corners of an item, which its fill and its state layer follow.
pub(crate) const CORNER: f32 = 12.0;

/// Whether an item is so, which may read signals.
pub(crate) type When = Rc<dyn Fn() -> bool>;

/// The fill of an item, if it is `selected`, in a list that has the focus or not.
pub fn selection_fill(c: &ColorScheme, selected: bool, focused: bool) -> Option<Color> {
    match (selected, focused) {
        (true, true) => Some(c.secondary_container),
        (true, false) => Some(
            c.surface_container_highest
                .mix(c.secondary_container, UNFOCUSED),
        ),
        (false, _) => None,
    }
}
