use anti_webui::prelude::*;
use anti_webui_md3e::components::tooltip;
use anti_webui_md3e::icons::{self, Symbol};
use playback::PlayerSnapshot;

use crate::control::{STANDARD, icon_control};
use crate::position::{HANDLE, flat_line, handle, lane};

const WIDTH: f32 = 96.0;
const LOCKED: &str = "Set the volume on the DAC while playing DSD";

/// Mute and the level. The level is kept for the next launch once it is let go.
pub(crate) fn volume(player: playback::Player, shown: Signal<PlayerSnapshot>) -> Div {
    let is_locked = memo(move || shown.with(PlayerSnapshot::is_volume_locked));
    div().flex().flex_none().child(dyn_view(move || {
        let is_locked = is_locked.get();
        untrack(|| {
            let controls = controls(player.clone(), shown, is_locked);
            // What the screen cannot tell: why nothing reacts
            if is_locked {
                tooltip(controls, LOCKED)
            } else {
                controls
            }
        })
    }))
}

fn controls(player: playback::Player, shown: Signal<PlayerSnapshot>, is_locked: bool) -> Div {
    let is_silent = memo(move || shown.with(PlayerSnapshot::is_silent));
    let level = memo(move || shown.with(PlayerSnapshot::volume));
    let symbol = move || -> &'static Symbol {
        if is_silent.get() {
            icons::VOLUME_OFF
        } else if level.get() < 0.5 {
            icons::VOLUME_DOWN
        } else {
            icons::VOLUME_UP
        }
    };
    let muting = player.clone();
    let mute = icon_control(symbol, STANDARD, || false, move || is_locked)
        .access(move |a| a.label(if is_silent.get() { "Unmute" } else { "Mute" }))
        .on_click(move |_| muting.toggle_mute());

    let can_move = move || !is_locked;
    let (by_pointer, by_key) = (player.clone(), player.clone());
    let value = Binding::new(move || level.get(), move |level| player.set_volume(level));
    let options = SliderOptions::new().inset(HANDLE / 2.0);
    let slider = slider_with(value, 0.0, 1.0, options, move |state| {
        let fraction = move || state.fraction();
        lane(can_move)
            .child(flat_line(state, fraction, can_move))
            .child(handle(state, fraction, can_move))
    })
    .w(WIDTH)
    .flex_none()
    .disabled(move || is_locked)
    .access(move |a| {
        let percent = (level.get() * 100.0).round();
        a.label("Volume").value_text(format!("{percent}%"))
    })
    .on_grab(move |grab| {
        if matches!(grab.phase, GrabPhase::End | GrabPhase::Cancel) {
            by_pointer.save_volume();
        }
    })
    .on_key_up(move |key| {
        if is_slider_key(&key.key) {
            by_key.save_volume();
        }
    });
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(4.0_f32)
        .child(mute)
        .child(slider)
}

fn is_slider_key(key: &Key) -> bool {
    matches!(
        key,
        Key::Named(
            NamedKey::ArrowLeft
                | NamedKey::ArrowRight
                | NamedKey::ArrowUp
                | NamedKey::ArrowDown
                | NamedKey::PageUp
                | NamedKey::PageDown
                | NamedKey::Home
                | NamedKey::End
        )
    )
}
