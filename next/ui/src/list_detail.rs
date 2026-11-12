//! A list beside what is chosen in it, on panes of their own: the list keeps the width it is given
//! and the detail takes the rest.

use anti_webui::prelude::*;

use crate::pane::{GAP, pane};
use crate::pane_handle::{Resizes, pane_handle};

/// The list's width at first.
pub const LIST_WIDTH: f32 = 320.0;
pub const LIST_MIN_WIDTH: f32 = 240.0;
pub const LIST_MAX_WIDTH: f32 = 480.0;
/// The least the detail keeps while the list is made wider or the window narrower. Down to the
/// narrowest window, with the rail closed, this leaves the list its least.
pub const DETAIL_MIN_WIDTH: f32 = 400.0;

/// The width the user gave the list, which the owner keeps for as long as the split should last.
/// Cheap to copy.
#[derive(Clone, Copy)]
pub struct Split {
    list_width: Signal<f32>,
}

impl Default for Split {
    fn default() -> Self {
        Self::new()
    }
}

impl Split {
    pub fn new() -> Self {
        Self::with_list_width(LIST_WIDTH)
    }

    /// Starting from the width the user gave the list before.
    pub fn with_list_width(list_width: f32) -> Self {
        Self {
            list_width: signal(list_width),
        }
    }

    /// What the user asked for, which a narrow window can make the list narrower than. Tracks it.
    pub fn list_width(&self) -> f32 {
        self.list_width.get()
    }
}

/// How wide the list shows when it is asked to be `wanted` wide in `room`: within its bounds, and
/// narrower before the detail gets narrower than its least.
pub fn list_width_in(wanted: f32, room: f32) -> f32 {
    let most = (room - GAP - DETAIL_MIN_WIDTH).min(LIST_MAX_WIDTH);
    wanted.min(most).max(LIST_MIN_WIDTH)
}

/// `list` and `detail` side by side on two panes, with a handle between them that drags and takes
/// Left, Right, Home and End to change the list's width. It takes the room it is given.
pub fn list_detail(split: Split, list: Div, detail: Div) -> Div {
    let room = signal(0.0f32);
    let shown = memo(move || list_width_in(split.list_width.get(), room.get()));
    div()
        .flex()
        .flex_row()
        .min_w(0)
        .min_h(0)
        .on_resized(move |width, _| room.set_if_changed(width))
        .child(pane(list).flex_none().dyn_style(move |s| s.w(shown.get())))
        .child(pane_handle(
            "List Width",
            Resizes::Before,
            move || shown.get(),
            LIST_MIN_WIDTH,
            move || list_width_in(LIST_MAX_WIDTH, room.get()),
            move |width| split.list_width.set_if_changed(width),
        ))
        .child(pane(detail).flex_1())
}

#[cfg(test)]
mod tests;
