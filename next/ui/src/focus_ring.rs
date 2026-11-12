//! The ring around what has the focus from the keyboard, as md3e's components draw it, for what is
//! made pressable here. Each is used from a `dyn_style`.

use anti_webui::prelude::*;
use anti_webui_md3e::ColorScheme;

/// As wide as md3e's focus ring.
const WIDTH: f32 = 3.0;
const OFFSET: f32 = 2.0;

/// `s` with the ring, outside its edge, while `focused`.
pub fn focus_ring(s: Style, focused: bool, c: &ColorScheme) -> Style {
    ring(s, focused, OFFSET, c)
}

/// `s` with the ring inside its edge while `focused`, for an item of a list, which clips it and
/// whose next item would cover it.
pub fn inner_focus_ring(s: Style, focused: bool, c: &ColorScheme) -> Style {
    ring(s, focused, -WIDTH, c)
}

fn ring(s: Style, focused: bool, offset: f32, c: &ColorScheme) -> Style {
    if focused {
        s.outline(WIDTH)
            .outline_offset(offset)
            .outline_color(c.secondary)
    } else {
        s
    }
}
