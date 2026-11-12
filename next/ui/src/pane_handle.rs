//! The handle in the gap between two panes, which makes one of them wider or narrower.

use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::motion::{fast_effects, fast_spatial};

use crate::pane::GAP;

/// How far a press of Left or Right moves the handle.
pub const KEY_STEP: f32 = 16.0;

/// The handle of Material's panes: a pill in the gap between them, larger while held.
const HANDLE: (f32, f32) = (4.0, 48.0);
const HELD_HANDLE: (f32, f32) = (12.0, 52.0);

/// Which of the two panes beside a handle it makes wider or narrower.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resizes {
    Before,
    After,
}

/// A handle as wide as the gap between two panes, which drags, and takes Left, Right, Home and
/// End, to change the width of the pane it `resizes`. `label` names it. `width` is the pane's
/// width now, which goes from `least` to what `most` gives; both may read signals. `set` is asked
/// for a width in those bounds.
pub fn pane_handle(
    label: &'static str,
    resizes: Resizes,
    width: impl Fn() -> f32 + Copy + 'static,
    least: f32,
    most: impl Fn() -> f32 + Copy + 'static,
    set: impl Fn(f32) + Copy + 'static,
) -> Div {
    let hovered = signal(false);
    let held = signal(false);
    let keyed = signal(false);
    let pill_width = follow(
        move || if held.get() { HELD_HANDLE.0 } else { HANDLE.0 },
        fast_spatial(),
    );
    let pill_height = follow(
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
    let ask = move |wanted: f32| set(wanted.min(untrack(most)).max(least));
    // Toward the pane it resizes is narrower
    let wider = match resizes {
        Resizes::Before => 1.0,
        Resizes::After => -1.0,
    };
    let pill = div()
        .flex_none()
        .rounded(HELD_HANDLE.0 / 2.0)
        .dyn_style(move |s| {
            let c = color_scheme();
            let s = s
                .w(pill_width.get())
                .h(pill_height.get())
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
            a.role(Role::Separator)
                .label(label)
                .value(width() as f64, least as f64, most() as f64)
        })
        .on_hover(move |on| hovered.set(on))
        .on_active(move |on| held.set(on))
        .on_focus_visible(move |on| keyed.set(on))
        .grab_threshold(1.0_f32)
        .on_grab(move |g| match g.phase {
            GrabPhase::Start => from.set(untrack(width)),
            GrabPhase::Move => ask(from.get_untracked() + wider * g.total.x),
            GrabPhase::End | GrabPhase::Cancel => {}
        })
        .on_key_down(move |e| {
            let now = untrack(width);
            let to = match &e.key {
                Key::Named(NamedKey::ArrowLeft) => now - wider * KEY_STEP,
                Key::Named(NamedKey::ArrowRight) => now + wider * KEY_STEP,
                Key::Named(NamedKey::Home) => least,
                Key::Named(NamedKey::End) => untrack(most),
                _ => return,
            };
            if e.modifiers.shift || e.modifiers.ctrl || e.modifiers.alt || e.modifiers.logo {
                return;
            }
            e.stop_propagation();
            e.prevent_default();
            ask(to);
        })
        .child(pill)
}

#[cfg(test)]
mod tests;
