//! How the items of a list or a grid look when selected and when the keys are on them.
//!
//! A selected item is filled, more strongly while the keys go to its list. Its words keep the
//! colors they have unselected: md3e's `on_secondary_container` is weaker than the
//! `on_surface_variant` of the details, and a title in it would read below them.
//!
//! What has the focus is the list, not an item, so the item the keys are on shows it as md3e's
//! components show theirs: their focus layer and their ring. The ring is inside the item's edge,
//! where neither the list, which cuts what goes past its own, nor the items next to it cover it.

use std::rc::Rc;

use anti_webui::prelude::*;
use anti_webui_md3e::ColorScheme;
use anti_webui_md3e::components::Ink;

/// How much of the focused selection's fill an unfocused selection has, over the fill of a card at
/// rest. Less, and in a dark scheme it looks as a row the pointer is over does.
const UNFOCUSED: f32 = 0.5;
/// As wide as md3e's focus ring.
const RING: f32 = 3.0;

/// The corners of an item, which its fill, its state layer and its ring follow.
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

/// md3e's focus ring, inside the edge, while the keys are on the item.
pub fn focus_ring(s: Style, keyed: bool, c: &ColorScheme) -> Style {
    if keyed {
        s.outline(RING)
            .outline_offset(-RING)
            .outline_color(c.secondary)
    } else {
        s
    }
}

/// Makes `ink`, which paints an item, show its focus layer while `keyed` says the keys are on the
/// item.
pub(crate) fn shows_keys_on(ink: &Ink, keyed: Memo<bool>) {
    let focused = ink.focused;
    effect(move || focused.set(keyed.get()));
}
