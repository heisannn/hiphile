use std::rc::Rc;

use anti_webui::prelude::*;
use anti_webui_md3e::components::{AlertDialog, alert_dialog, text_button};
use hiphile_core::settings::{AudioSettings, AudioSnapshot, Choice, DsdWarning};

use crate::choice_field::choice_of;
use crate::controls::{never, radios, switch_of, when};
use crate::group::{Group, Row, When};

type Options = fn(&AudioSnapshot) -> Vec<Choice>;
type Value = fn(&AudioSnapshot) -> String;
type Warn = fn(&AudioSnapshot, &str) -> Option<DsdWarning>;
type Set = fn(&AudioSettings, &str);

/// The groups of the Audio page about DSD, which stay out of the way until it is on.
pub(crate) fn dsd_groups(audio: &AudioSettings, shown: Signal<AudioSnapshot>) -> Vec<View> {
    if !AudioSettings::is_dsd_offered() {
        return Vec::new();
    }
    let is_on = move || shown.with(AudioSnapshot::is_dsd_on);
    let (filter, modulator) = (filter_group(audio, shown), modulator_group(audio, shown));
    vec![
        dsd_group(audio, shown).view().into_view(),
        show(is_on, move || filter.clone().view()).into_view(),
        show(is_on, move || modulator.clone().view()).into_view(),
    ]
}

/// A row with the dropdown of one of the DSD settings.
fn choice(
    audio: &AudioSettings,
    shown: Signal<AudioSnapshot>,
    headline: &'static str,
    (options, value, set): (Options, Value, Set),
    supporting: &'static str,
) -> Row {
    let audio = audio.clone();
    Row::new(headline, move || {
        choice_of(headline, &audio, shown, options, value, set, never())
    })
    .supporting_text(supporting)
}

fn dsd_group(audio: &AudioSettings, shown: Signal<AudioSnapshot>) -> Group {
    let is_on = move || shown.with(AudioSnapshot::is_dsd_on);
    let blocked = when(move || shown.with(|shown| shown.dsd_obstacle().is_some()));
    let format = {
        let audio = audio.clone();
        let off = blocked.clone();
        Row::new("Output format", move || {
            asking_radios(
                "Output format",
                &audio,
                shown,
                AudioSnapshot::dsd_format_options,
                AudioSnapshot::dsd_format_value,
                AudioSnapshot::dsd_format_warning,
                AudioSettings::set_dsd_format,
                off.clone(),
            )
        })
        .supporting(move || shown.with(|shown| shown.dsd_obstacle().map(str::to_owned)))
        .disabled_when(move || blocked())
    };
    let base_rate = {
        let audio = audio.clone();
        Row::new("Base rate", move || {
            asking_radios(
                "Base rate",
                &audio,
                shown,
                AudioSnapshot::dsd_base_rate_options,
                AudioSnapshot::dsd_base_rate_value,
                AudioSnapshot::dsd_base_rate_warning,
                AudioSettings::set_dsd_base_rate,
                never(),
            )
        })
        .supporting_text(
            "Standard DSD rates are based on 44.1 kHz. Match each song plays 48 kHz songs at \
             rates based on 48 kHz, which some DACs can't decode.",
        )
    };
    let fade_in = {
        let audio = audio.clone();
        Row::new("Fade in", move || {
            switch_of(
                "Fade in",
                &audio,
                shown,
                AudioSnapshot::fades_dsd_in,
                AudioSettings::set_fades_dsd_in,
                never(),
            )
        })
        .supporting_text("Rises over 3 seconds when DSD turns on or its settings change.")
    };
    Group::titled("DSD").rows([
        format,
        choice(
            audio,
            shown,
            "DSD rate",
            (
                AudioSnapshot::dsd_rate_options,
                AudioSnapshot::dsd_rate_value,
                AudioSettings::set_dsd_rate,
            ),
            "Higher rates move noise further above hearing and use more CPU.",
        )
        .visible_when(is_on),
        base_rate.visible_when(is_on),
        fade_in.visible_when(is_on),
        choice(
            audio,
            shown,
            "Quality",
            (
                AudioSnapshot::dsd_quality_options,
                AudioSnapshot::dsd_quality_value,
                AudioSettings::set_dsd_quality,
            ),
            "Higher settings use more CPU. Changing a value below makes this Custom.",
        )
        .visible_when(is_on),
    ])
}

fn filter_group(audio: &AudioSettings, shown: Signal<AudioSnapshot>) -> Group {
    let sinc = move || shown.with(AudioSnapshot::dsd_uses_sinc);
    Group::titled("DSD filter").rows([
        choice(
            audio,
            shown,
            "Filter",
            (
                AudioSnapshot::dsd_filter_options,
                AudioSnapshot::dsd_filter_value,
                AudioSettings::set_dsd_filter,
            ),
            "The method that raises the song to the DSD rate. FFT takes much more memory at \
             large sizes.",
        ),
        choice(
            audio,
            shown,
            "Length",
            (
                AudioSnapshot::dsd_sinc_length_options,
                AudioSnapshot::dsd_sinc_length_value,
                AudioSettings::set_dsd_sinc_length,
            ),
            "Longer filters cut off more steeply and use more CPU.",
        )
        .visible_when(sinc),
        choice(
            audio,
            shown,
            "FFT size",
            (
                AudioSnapshot::dsd_fft_size_options,
                AudioSnapshot::dsd_fft_size_value,
                AudioSettings::set_dsd_fft_size,
            ),
            "Larger sizes cut off more steeply and use more memory.",
        )
        .visible_when(move || !sinc()),
        choice(
            audio,
            shown,
            "Window",
            (
                AudioSnapshot::window_options,
                AudioSnapshot::dsd_window_value,
                AudioSettings::set_dsd_window,
            ),
            "The filter's shape. Further down the list removes more unwanted sound but cuts off \
             less steeply.",
        ),
        choice(
            audio,
            shown,
            "Cutoff",
            (
                AudioSnapshot::dsd_cutoff_options,
                AudioSnapshot::dsd_cutoff_value,
                AudioSettings::set_dsd_cutoff,
            ),
            "Where the filter starts removing highs, as a fraction of the song's top frequency. \
             Higher keeps more treble and lets through more unwanted sound.",
        )
        .visible_when(sinc),
    ])
}

fn modulator_group(audio: &AudioSettings, shown: Signal<AudioSnapshot>) -> Group {
    let narrow = when(move || !shown.with(AudioSnapshot::is_dsd_noise_band_widenable));
    let noise_band = {
        let audio = audio.clone();
        let off = narrow.clone();
        Row::new("Noise band", move || {
            choice_of(
                "Noise band",
                &audio,
                shown,
                AudioSnapshot::dsd_noise_band_options,
                AudioSnapshot::dsd_noise_band_value,
                AudioSettings::set_dsd_noise_band,
                off.clone(),
            )
        })
        .supporting_text(
            "The range kept quiet, with the noise moved above it. Widening it to 48 kHz adds \
             noise below 20 kHz, too much at rates under DSD256.",
        )
        .disabled_when(move || narrow())
    };
    let zeros = {
        let audio = audio.clone();
        Row::new("Optimized zeros", move || {
            switch_of(
                "Optimized zeros",
                &audio,
                shown,
                AudioSnapshot::has_optimized_dsd_zeros,
                AudioSettings::set_optimized_dsd_zeros,
                never(),
            )
        })
        .supporting_text(
            "Spreads the quietest points across that range instead of stacking them at its low \
             end, which lowers the noise overall.",
        )
    };
    Group::titled("DSD modulator").rows([
        choice(
            audio,
            shown,
            "Order",
            (
                AudioSnapshot::dsd_order_options,
                AudioSnapshot::dsd_order_value,
                AudioSettings::set_dsd_order,
            ),
            "How steeply noise is pushed above hearing. Higher takes more CPU and is not always \
             quieter.",
        ),
        choice(
            audio,
            shown,
            "Noise gain",
            (
                AudioSnapshot::dsd_noise_gain_options,
                AudioSnapshot::dsd_noise_gain_value,
                AudioSettings::set_dsd_noise_gain,
            ),
            "How hard noise is pushed above hearing. Higher lowers audible noise, but above 1.40 \
             loud music can stop playback unless the Limiter is at 0 dBTP or lower.",
        ),
        noise_band,
        zeros,
        choice(
            audio,
            shown,
            "Dither",
            (
                AudioSnapshot::dsd_dither_options,
                AudioSnapshot::dsd_dither_value,
                AudioSettings::set_dsd_dither,
            ),
            "Adds faint random noise, which can break up tones the modulator makes in very quiet \
             passages.",
        ),
        choice(
            audio,
            shown,
            "Input level",
            (
                AudioSnapshot::dsd_input_level_options,
                AudioSnapshot::dsd_input_level_value,
                AudioSettings::set_dsd_input_level,
            ),
            "Changes the song's level before conversion. Lower it if loud songs stop playback.",
        ),
        choice(
            audio,
            shown,
            "Limiter",
            (
                AudioSnapshot::dsd_limiter_options,
                AudioSnapshot::dsd_limiter_value,
                AudioSettings::set_dsd_limiter,
            ),
            "Turns down peaks above this level, measured in true peak (dBTP), so loud songs \
             don't overload the modulator. With Off, they can stop playback.",
        ),
    ])
}

/// A choice made only once the user agrees to what `warning` says about it, where it says
/// something.
struct Asked {
    warning: DsdWarning,
    chosen: String,
}

/// Radios for the choices that `options` reads, which ask first when the settings warn of one.
#[allow(clippy::too_many_arguments)]
fn asking_radios(
    name: &'static str,
    audio: &AudioSettings,
    shown: Signal<AudioSnapshot>,
    options: Options,
    value: Value,
    warning: Warn,
    set: Set,
    disabled: When,
) -> Div {
    let options = memo(move || shown.with(options));
    let audio = audio.clone();
    div().child(dyn_view(move || {
        let choices = options.get();
        let (audio, disabled) = (audio.clone(), disabled.clone());
        untrack(move || radios_asking(name, &audio, shown, choices, value, warning, set, disabled))
    }))
}

#[allow(clippy::too_many_arguments)]
fn radios_asking(
    name: &'static str,
    audio: &AudioSettings,
    shown: Signal<AudioSnapshot>,
    choices: Vec<Choice>,
    value: Value,
    warning: Warn,
    set: Set,
    disabled: When,
) -> Div {
    let asked = signal(None::<Rc<Asked>>);
    let open = signal(false);
    let labels = choices
        .iter()
        .map(|choice| choice.label().to_owned())
        .collect();
    let values: Rc<[String]> = choices
        .iter()
        .map(|choice| choice.value().to_owned())
        .collect();
    let selected = {
        let values = values.clone();
        move || {
            let value = shown.with(value);
            values.iter().position(|v| *v == value).unwrap_or_default()
        }
    };
    let choose = {
        let audio = audio.clone();
        move |ix: usize| {
            let Some(chosen) = values.get(ix).cloned() else {
                return;
            };
            let settings = audio.snapshot();
            if value(&settings) == chosen {
                return;
            }
            match warning(&settings, &chosen) {
                Some(warning) => {
                    asked.set(Some(Rc::new(Asked { warning, chosen })));
                    open.set(true);
                }
                None => set(&audio, &chosen),
            }
        }
    };
    let audio = audio.clone();
    let dialog = dyn_view(move || {
        let asked = asked.get();
        let audio = audio.clone();
        untrack(move || {
            asked.map(|asked| {
                let Asked { warning, chosen } = &*asked;
                let (confirm_label, chosen) = (warning.confirm_label().to_owned(), chosen.clone());
                let audio = audio.clone();
                alert_dialog(
                    move || open.get(),
                    move || open.set(false),
                    AlertDialog::new()
                        .title(warning.title())
                        .text(warning.message())
                        .dismiss(move || text_button("Cancel").on_click(move |_| open.set(false)))
                        .confirm(move || {
                            let (audio, chosen) = (audio.clone(), chosen.clone());
                            text_button(confirm_label.as_str()).on_click(move |_| {
                                open.set(false);
                                set(&audio, &chosen);
                            })
                        }),
                )
            })
        })
    });
    radios(name, labels, selected, choose, disabled).child(dialog)
}
