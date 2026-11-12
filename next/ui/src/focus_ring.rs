//! The ring around what has the focus from the keyboard, as md3e's components draw it, for what is
//! made pressable here.

use anti_webui::prelude::*;
use anti_webui_md3e::ColorScheme;

/// `s` with the ring while `focused`, to be used from a `dyn_style`.
pub fn focus_ring(s: Style, focused: bool, c: &ColorScheme) -> Style {
    if focused {
        s.outline(3.0_f32)
            .outline_offset(2.0_f32)
            .outline_color(c.secondary)
    } else {
        s
    }
}
