use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::components::{indeterminate_circular_progress, wavy_progress_eased};
use anti_webui_md3e::typography::TextStyle;
use hiphile_core::format::format_duration;
use next_ui::focus_ring::focus_ring;
use playback::PlayerSnapshot;

use crate::reading::keep_reading;

/// How often the position is read while there is a song: the core does not tell of it.
pub(crate) const REFRESH: Duration = Duration::from_millis(250);
/// How long the wavy line takes to come to a new place, short enough to follow a drag.
const GLIDE: Duration = Duration::from_millis(100);
pub(crate) const HANDLE: f32 = 4.0;
const HANDLE_HEIGHT: f32 = 24.0;
/// How tall a slider of this crate is, which its handle fills.
pub(crate) const LANE: f32 = 24.0;
const STROKE: f32 = 4.0;
/// Between the line and the track, and between the handle and either.
const GAP: f32 = 4.0;
/// Each time has a width of its own, so that the line keeps still as the digits change.
const TIME: f32 = 44.0;

/// Dragging along the line shows where the song would go, and seeks there when let go; the keys
/// seek at once. While `is_covered`, the line keeps still, as nothing of it can be seen.
pub(crate) fn position_line(
    player: playback::Player,
    shown: Signal<PlayerSnapshot>,
    is_covered: impl Fn() -> bool + 'static,
) -> Div {
    let has_song = memo(move || shown.with(PlayerSnapshot::has_song));
    let duration = memo(move || shown.with(PlayerSnapshot::duration));
    let is_seekable = memo(move || shown.with(PlayerSnapshot::is_seekable));
    let waves = memo(move || shown.with(PlayerSnapshot::is_playing) && !is_covered());
    let is_buffering = memo(move || shown.with(PlayerSnapshot::is_buffering));
    let position = signal(player.position());
    keep_reading(player.clone(), position, REFRESH, move || has_song.get());

    // Where a drag has got to, as a fraction of the song
    let scrub = signal(None::<f32>);
    let is_held = Rc::new(Cell::new(false));
    let entry = memo(move || shown.with(|shown| shown.queue().current_id()));
    effect(move || {
        entry.get();
        untrack(|| scrub.set(None));
    });
    let fraction = move || {
        scrub.get().unwrap_or_else(|| match duration.get() {
            Some(duration) => (position.get().as_secs_f32() / duration.as_secs_f32()).min(1.0),
            None => 0.0,
        })
    };
    let shown_position = move || match (scrub.get(), duration.get()) {
        (Some(fraction), Some(duration)) => duration.mul_f32(fraction),
        _ => position.get(),
    };
    let seek = move |fraction: f32| {
        let target = untrack(|| duration.get()).map(|duration| duration.mul_f32(fraction));
        if let Some(target) = target.and_then(|target| player.seek_to(target)) {
            position.set(target);
        }
    };
    let value = {
        let (is_held, seek) = (is_held.clone(), seek.clone());
        Binding::new(fraction, move |fraction| {
            if !untrack(|| is_seekable.get()) {
                return;
            }
            if is_held.get() {
                scrub.set(Some(fraction));
            } else {
                seek(fraction);
            }
        })
    };
    let options = SliderOptions::new().inset(HANDLE / 2.0);
    let seek_bar = slider_with(value, 0.0, 1.0, options, move |state| {
        let can_move = move || is_seekable.get();
        let line = dyn_view(move || {
            if waves.get() {
                untrack(|| wavy_line(fraction))
            } else {
                untrack(|| flat_line(state, fraction, can_move))
            }
        });
        lane(can_move)
            .child(line)
            .child(handle(state, fraction, can_move))
            // Before the slider's own, so that what the press moves to waits for the release
            .on_grab(move |grab| match grab.phase {
                GrabPhase::Start => is_held.set(true),
                GrabPhase::Move => {}
                GrabPhase::End => {
                    is_held.set(false);
                    if let Some(fraction) = scrub.get_untracked() {
                        scrub.set(None);
                        seek(fraction);
                    }
                }
                GrabPhase::Cancel => {
                    is_held.set(false);
                    scrub.set(None);
                }
            })
    })
    .flex_1()
    .min_w(0.0_f32)
    .disabled(move || !is_seekable.get())
    .access(move |a| {
        let told = match duration.get() {
            Some(duration) => format!(
                "{} of {}",
                format_duration(shown_position()),
                format_duration(duration)
            ),
            None => format_duration(shown_position()),
        };
        a.label("Position").value_text(told)
    });

    let elapsed = div()
        .relative()
        .flex()
        .justify_end()
        .child(
            time(move || format_duration(shown_position()))
                .dyn_style(move |s| s.opacity(if is_buffering.get() { 0.0_f32 } else { 1.0 })),
        )
        .child(show(
            move || is_buffering.get(),
            || {
                div()
                    .absolute()
                    .inset(0.0_f32)
                    .flex()
                    .items_center()
                    .justify_end()
                    .child(
                        indeterminate_circular_progress()
                            .w(16.0_f32)
                            .h(16.0_f32)
                            .access(|a| a.role(Role::ProgressBar).label("Buffering")),
                    )
            },
        ));
    let total = time(move || duration.get().map(format_duration).unwrap_or_default());
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(8.0_f32)
        .h(LANE)
        .child(elapsed.w(TIME).flex_none())
        .child(seek_bar)
        .child(total.w(TIME).flex_none())
}

fn time(content: impl IntoTextSource) -> Div {
    TextStyle::LABEL_MEDIUM
        .apply(div())
        .whitespace_nowrap()
        .dyn_style(|s| s.text_color(color_scheme().on_surface_variant))
        .child(text(content))
}

/// What a slider of this crate is drawn in.
pub(crate) fn lane(can_move: impl Fn() -> bool + 'static) -> Div {
    div()
        .relative()
        .flex()
        .items_center()
        .h(LANE)
        .dyn_style(move |s| {
            if can_move() {
                s.cursor(CursorIcon::Pointer)
            } else {
                s.cursor(CursorIcon::Default)
            }
        })
}

/// The line of a song that plays. It has no wave near either end.
fn wavy_line(fraction: impl Fn() -> f32 + 'static) -> Div {
    wavy_progress_eased(fraction, Transition::new(GLIDE).ease_out())
        .w_full()
        .access(|a| a.hidden())
}

/// A straight line up to `fraction` and the track after it, apart by a gap that makes room for
/// the handle while it shows. Unlike `linear_progress` of `anti_webui_md3e`, which comes to a new
/// value in most of a second, it keeps up with a drag.
pub(crate) fn flat_line(
    state: SliderState,
    fraction: impl Fn() -> f32 + Copy + 'static,
    can_move: impl Fn() -> bool + Copy + 'static,
) -> Div {
    let is_up = move || is_handle_up(state, can_move);
    let line = div()
        .absolute()
        .left(0.0_f32)
        .top((LANE - STROKE) / 2.0)
        .h(STROKE)
        .rounded(STROKE / 2.0)
        .dyn_style(move |s| {
            let t = fraction();
            // Up to the handle's left side, which is `HANDLE * t` before the end of the line
            let short = if is_up() { HANDLE * t + GAP } else { 0.0 };
            s.bg(color_scheme().primary)
                .right(pct((1.0 - t) * 100.0))
                .mr(short)
                .opacity(if t > 0.0 { 1.0_f32 } else { 0.0 })
        });
    let track = div()
        .absolute()
        .right(0.0_f32)
        .top((LANE - STROKE) / 2.0)
        .h(STROKE)
        .rounded(STROKE / 2.0)
        .dyn_style(move |s| {
            let t = fraction();
            let after = if is_up() {
                HANDLE - HANDLE * t + GAP
            } else {
                GAP
            };
            s.bg(color_scheme().secondary_container)
                .left(pct(t * 100.0))
                .ml(after)
                .opacity(if t < 1.0 { 1.0_f32 } else { 0.0 })
        });
    div()
        .relative()
        .w_full()
        .h(LANE)
        .access(|a| a.hidden())
        .child(line)
        .child(track)
}

fn is_handle_up(state: SliderState, can_move: impl Fn() -> bool) -> bool {
    can_move() && (state.hovered() || state.dragging() || state.focus_visible())
}

/// The handle of a slider in a [`lane`] whose ends are half a handle in from its box, as
/// `SliderOptions::inset(HANDLE / 2.0)` puts them. It shows while the pointer or the focus is on
/// the slider.
pub(crate) fn handle(
    state: SliderState,
    fraction: impl Fn() -> f32 + Copy + 'static,
    can_move: impl Fn() -> bool + Copy + 'static,
) -> Show {
    show(
        move || is_handle_up(state, can_move),
        move || {
            div()
                .absolute()
                .top((LANE - HANDLE_HEIGHT) / 2.0)
                .w(HANDLE)
                .h(HANDLE_HEIGHT)
                .rounded(HANDLE / 2.0)
                .pointer_events_none()
                .dyn_style(move |s| {
                    let c = color_scheme();
                    let t = fraction();
                    let s = s.bg(c.primary).left(pct(t * 100.0)).ml(-HANDLE * t);
                    focus_ring(s, state.focus_visible(), &c)
                })
        },
    )
}

#[cfg(test)]
mod tests;
