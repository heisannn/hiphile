//! A list beside what is chosen in it, on panes of their own: the list keeps the width it is given
//! and the detail takes the rest.

use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::motion::{fast_effects, fast_spatial};

use crate::pane::{GAP, pane};

/// The list's width at first.
pub const LIST_WIDTH: f32 = 320.0;
pub const LIST_MIN_WIDTH: f32 = 240.0;
pub const LIST_MAX_WIDTH: f32 = 480.0;
/// The least the detail keeps while the list is made wider or the window narrower. Down to the
/// narrowest window, with the rail closed, this leaves the list its least.
pub const DETAIL_MIN_WIDTH: f32 = 400.0;
/// How far a press of Left or Right moves the divider.
const KEY_STEP: f32 = 16.0;

/// The handle of Material's panes: a pill in the gap between them, larger while held.
const HANDLE: (f32, f32) = (4.0, 48.0);
const HELD_HANDLE: (f32, f32) = (12.0, 52.0);

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
        Self {
            list_width: signal(LIST_WIDTH),
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
        .child(handle(split, shown, room))
        .child(pane(detail).flex_1())
}

fn handle(split: Split, shown: Memo<f32>, room: Signal<f32>) -> Div {
    let hovered = signal(false);
    let held = signal(false);
    let keyed = signal(false);
    let width = follow(
        move || if held.get() { HELD_HANDLE.0 } else { HANDLE.0 },
        fast_spatial(),
    );
    let height = follow(
        move || if held.get() { HELD_HANDLE.1 } else { HANDLE.1 },
        fast_spatial(),
    );
    let strong = follow(
        move || {
            if held.get() || hovered.get() {
                1.0f32
            } else {
                0.0
            }
        },
        fast_effects(),
    );
    let from = signal(0.0f32);
    let set = move |wanted: f32| {
        let most = list_width_in(LIST_MAX_WIDTH, room.get_untracked());
        split
            .list_width
            .set_if_changed(wanted.clamp(LIST_MIN_WIDTH, most));
    };
    let pill = div()
        .flex_none()
        .rounded(HELD_HANDLE.0 / 2.0)
        .dyn_style(move |s| {
            let c = color_scheme();
            let s = s
                .w(width.get())
                .h(height.get())
                .bg(c.outline.mix(c.on_surface, strong.get().clamp(0.0, 1.0)));
            if keyed.get() {
                s.outline(3.0_f32)
                    .outline_offset(2.0_f32)
                    .outline_color(c.secondary)
            } else {
                s
            }
        });
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .w(GAP)
        .cursor(CursorIcon::EwResize)
        .select_none()
        .focusable()
        .access(move |a| {
            a.role(Role::Separator).label("List Width").value(
                shown.get() as f64,
                LIST_MIN_WIDTH as f64,
                list_width_in(LIST_MAX_WIDTH, room.get()) as f64,
            )
        })
        .on_hover(move |on| hovered.set(on))
        .on_active(move |on| held.set(on))
        .on_focus_visible(move |on| keyed.set(on))
        .grab_threshold(1.0_f32)
        .on_grab(move |g| match g.phase {
            GrabPhase::Start => from.set(untrack(|| shown.get())),
            GrabPhase::Move => set(from.get_untracked() + g.total.x),
            GrabPhase::End | GrabPhase::Cancel => {}
        })
        .on_key_down(move |e| {
            let now = untrack(|| shown.get());
            let to = match &e.key {
                Key::Named(NamedKey::ArrowLeft) => now - KEY_STEP,
                Key::Named(NamedKey::ArrowRight) => now + KEY_STEP,
                Key::Named(NamedKey::Home) => LIST_MIN_WIDTH,
                Key::Named(NamedKey::End) => LIST_MAX_WIDTH,
                _ => return,
            };
            if e.modifiers.shift || e.modifiers.ctrl || e.modifiers.alt || e.modifiers.logo {
                return;
            }
            e.stop_propagation();
            e.prevent_default();
            set(to);
        })
        .child(pill)
}

#[cfg(test)]
mod tests;
