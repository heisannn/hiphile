//! The surfaces a screen's body is made of, apart from the rail and the bar.

use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;

/// Between two panes, and from the panes to the window's edges.
pub const GAP: f32 = 16.0;
pub const RADIUS: f32 = 28.0;
/// From a pane's edge to what is in it: the line its headings, cards and the words of its rows
/// start on.
pub const INSET: f32 = 16.0;

/// `content` on a pane of its own, which it fills and is cut to.
pub fn pane(content: Div) -> Div {
    div()
        .flex()
        .flex_col()
        .min_w(0)
        .min_h(0)
        .rounded(RADIUS)
        .overflow_hidden()
        .dyn_style(|s| s.bg(color_scheme().surface_container))
        .child(content.flex_1().min_h(0).w_full())
}
