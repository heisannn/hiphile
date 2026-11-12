use std::cell::Cell;
use std::rc::Rc;

use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::components::{SliderStyle, styled_slider};
use anti_webui_md3e::typography::TextStyle;
use hiphile_core::settings::DspSettings;

use crate::controls::{reset_button, resettable};

const NAME: &str = "Balance";
const WORDS: i32 = 120;

/// The balance on a slider that goes both ways from the center, with the channel it lowers.
///
/// Only this slider changes the balance, so what it shows never has to follow the settings.
pub(crate) fn balance_slider(dsp: &DspSettings) -> Div {
    let range = DspSettings::balance_range();
    let db = signal(dsp.snapshot().balance_db());
    let set = {
        let dsp = dsp.clone();
        Rc::new(move |to: f64| {
            dsp.set_balance(to);
            db.set_if_changed(dsp.snapshot().balance_db());
        })
    };
    // The slider's keys go a twentieth of the way at a time, and a balance goes by its own step;
    // what the slider's key set is put right afterwards from where it was
    let before = Rc::new(Cell::new(None::<f64>));
    let binding = {
        let (set, before) = (set.clone(), before.clone());
        Binding::new(
            move || db.get() as f32,
            move |to: f32| {
                before.set(Some(db.get_untracked()));
                set(f64::from(to));
            },
        )
    };
    let slider = styled_slider(
        binding,
        SliderStyle::new()
            .range(range.min() as f32..=range.max() as f32)
            .centered(),
    );
    let said = {
        let dsp = dsp.clone();
        TextStyle::BODY_MEDIUM
            .apply(div().w(WORDS).flex_none().child(text(move || {
                db.with(|_| ());
                dsp.snapshot().balance_text()
            })))
            .text_align(TextAlign::Right)
            .dyn_style(|s| s.text_color(color_scheme().on_surface_variant))
    };
    let stepped = before.clone();
    let control = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(12.0_f32)
        .access(|a| a.role(Role::Group).label(NAME))
        .on_key_down({
            let set = set.clone();
            move |e| {
                let from = stepped.take();
                let by = match e.key {
                    Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowDown) => -range.step(),
                    Key::Named(NamedKey::ArrowRight | NamedKey::ArrowUp) => range.step(),
                    _ => return,
                };
                if let Some(from) = from {
                    set(from + by);
                }
            }
        })
        .on_pointer_up(move |_| before.set(None))
        .child(slider)
        .child(said);
    resettable(
        control,
        reset_button(NAME, move || db.get() != 0.0, move || set(0.0)),
    )
}

#[cfg(test)]
mod tests;
