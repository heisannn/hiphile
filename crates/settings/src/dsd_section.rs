use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::{
    WindowExt as _,
    radio::{Radio, RadioGroup},
    setting::{SettingField, SettingGroup, SettingItem},
};
use gpui_kit::{App, SharedString, Window};
use hiphile_core::settings::{AudioSettings, AudioSnapshot, Choice, DsdWarning};

use crate::audio_options::dropdown;

/// The Audio page's DSD groups, which stay out of the way until it is on.
pub(crate) fn dsd_groups(audio: &AudioSettings) -> Vec<SettingGroup> {
    if !AudioSettings::is_dsd_offered() {
        return Vec::new();
    }
    let settings = audio.snapshot();
    let mut format = SettingItem::new("Output format", format_field(audio))
        .keywords(["dsd", "dop", "pcm"])
        .disabled(settings.dsd_obstacle().is_some());
    if let Some(obstacle) = settings.dsd_obstacle() {
        format = format.description(obstacle);
    }
    if !settings.is_dsd_on() {
        return vec![SettingGroup::new().title("DSD").items([format])];
    }
    vec![
        SettingGroup::new().title("DSD").items([
            format,
            SettingItem::new(
                "DSD rate",
                dropdown(
                    audio,
                    settings.dsd_rate_options(),
                    AudioSnapshot::dsd_rate_value,
                    AudioSettings::set_dsd_rate,
                ),
            )
            .description("Higher rates move noise further above hearing and use more CPU."),
            SettingItem::new("Base rate", base_rate_field(audio))
                .description(
                    "Standard DSD rates are based on 44.1 kHz. Match each song plays 48 kHz songs \
                     at rates based on 48 kHz, which some DACs can't decode.",
                )
                .keywords(["dsd", "48 khz"]),
            SettingItem::new("Fade in", fade_in_field(audio))
                .description("Rises over 3 seconds when DSD turns on or its settings change.")
                .keywords(["dsd", "volume", "level"]),
            SettingItem::new(
                "Quality",
                dropdown(
                    audio,
                    settings.dsd_quality_options(),
                    AudioSnapshot::dsd_quality_value,
                    AudioSettings::set_dsd_quality,
                ),
            )
            .description("Higher settings use more CPU. Changing a value below makes this Custom.")
            .keywords(["dsd"]),
        ]),
        filter_group(audio),
        modulator_group(audio),
    ]
}

fn format_field(audio: &AudioSettings) -> SettingField<SharedString> {
    let audio = audio.clone();
    SettingField::render(move |_, _, _| {
        let settings = audio.snapshot();
        choice_group(
            "output-format",
            &audio,
            settings.dsd_format_options(),
            settings.dsd_format_value(),
            AudioSnapshot::dsd_format_warning,
            AudioSettings::set_dsd_format,
        )
        .disabled(settings.dsd_obstacle().is_some())
    })
}

fn base_rate_field(audio: &AudioSettings) -> SettingField<SharedString> {
    let audio = audio.clone();
    SettingField::render(move |_, _, _| {
        let settings = audio.snapshot();
        choice_group(
            "dsd-base-rate",
            &audio,
            settings.dsd_base_rate_options(),
            settings.dsd_base_rate_value(),
            AudioSnapshot::dsd_base_rate_warning,
            AudioSettings::set_dsd_base_rate,
        )
    })
}

/// Radios for `choices`, which ask first when the settings warn of a choice.
fn choice_group(
    id: &'static str,
    audio: &AudioSettings,
    choices: Vec<Choice>,
    value: String,
    warning: fn(&AudioSnapshot, &str) -> Option<DsdWarning>,
    set: fn(&AudioSettings, &str),
) -> RadioGroup {
    let selected = choices.iter().position(|choice| choice.value() == value);
    let radios = choices
        .iter()
        .enumerate()
        .map(|(ix, choice)| Radio::new((id, ix)).label(choice.label().to_owned()));
    let audio = audio.clone();
    RadioGroup::horizontal(id)
        .selected_index(selected)
        .children(radios)
        .on_click(move |ix, window, cx| {
            let Some(chosen) = choices.get(*ix).map(|choice| choice.value().to_owned()) else {
                return;
            };
            if chosen == value {
                return;
            }
            match warning(&audio.snapshot(), &chosen) {
                Some(warning) => confirm(&audio, warning, chosen, set, window, cx),
                None => set(&audio, &chosen),
            }
        })
}

fn confirm(
    audio: &AudioSettings,
    warning: DsdWarning,
    chosen: String,
    set: fn(&AudioSettings, &str),
    window: &mut Window,
    cx: &mut App,
) {
    let audio = audio.clone();
    let title = SharedString::from(warning.title().to_owned());
    let message = SharedString::from(warning.message().to_owned());
    let confirm_label = SharedString::from(warning.confirm_label().to_owned());
    window.open_alert_dialog(cx, move |alert, _, _| {
        let (audio, chosen) = (audio.clone(), chosen.clone());
        alert
            .title(title.clone())
            .description(message.clone())
            .ok_text(confirm_label.clone())
            .ok_variant(ButtonVariant::Danger)
            .show_cancel(true)
            .on_ok(move |_, _, _| {
                set(&audio, &chosen);
                true
            })
    });
}

fn fade_in_field(audio: &AudioSettings) -> SettingField<bool> {
    SettingField::switch(
        {
            let audio = audio.clone();
            move |_| audio.snapshot().fades_dsd_in()
        },
        {
            let audio = audio.clone();
            move |is_on, _| {
                audio.set_fades_dsd_in(is_on);
            }
        },
    )
}

fn filter_group(audio: &AudioSettings) -> SettingGroup {
    let settings = audio.snapshot();
    let mut items =
        vec![SettingItem::new(
        "Filter",
        dropdown(
            audio,
            settings.dsd_filter_options(),
            AudioSnapshot::dsd_filter_value,
            AudioSettings::set_dsd_filter,
        ),
    )
    .description(
        "The method that raises the song to the DSD rate. FFT takes much more memory at large \
         sizes.",
    )];
    if settings.dsd_uses_sinc() {
        items.push(
            SettingItem::new(
                "Length",
                dropdown(
                    audio,
                    settings.dsd_sinc_length_options(),
                    AudioSnapshot::dsd_sinc_length_value,
                    AudioSettings::set_dsd_sinc_length,
                ),
            )
            .description("Longer filters cut off more steeply and use more CPU."),
        );
    } else {
        items.push(
            SettingItem::new(
                "FFT size",
                dropdown(
                    audio,
                    settings.dsd_fft_size_options(),
                    AudioSnapshot::dsd_fft_size_value,
                    AudioSettings::set_dsd_fft_size,
                ),
            )
            .description("Larger sizes cut off more steeply and use more memory."),
        );
    }
    items.push(SettingItem::new(
        "Window",
        dropdown(
            audio,
            settings.window_options(),
            AudioSnapshot::dsd_window_value,
            AudioSettings::set_dsd_window,
        ),
    )
    .description(
        "The filter's shape. Further down the list removes more unwanted sound but cuts off less \
         steeply.",
    ));
    if settings.dsd_uses_sinc() {
        items.push(SettingItem::new(
            "Cutoff",
            dropdown(
                audio,
                settings.dsd_cutoff_options(),
                AudioSnapshot::dsd_cutoff_value,
                AudioSettings::set_dsd_cutoff,
            ),
        )
        .description(
            "Where the filter starts removing highs, as a fraction of the song's top frequency. \
             Higher keeps more treble and lets through more unwanted sound.",
        ));
    }
    SettingGroup::new()
        .title("DSD filter")
        .items(items.into_iter().map(|item| item.keywords(["dsd"])))
}

fn modulator_group(audio: &AudioSettings) -> SettingGroup {
    let settings = audio.snapshot();
    let zeros = SettingField::switch(
        {
            let audio = audio.clone();
            move |_| audio.snapshot().has_optimized_dsd_zeros()
        },
        {
            let audio = audio.clone();
            move |is_on, _| {
                audio.set_optimized_dsd_zeros(is_on);
            }
        },
    );
    SettingGroup::new().title("DSD modulator").items(
        [
            SettingItem::new(
                "Order",
                dropdown(
                    audio,
                    settings.dsd_order_options(),
                    AudioSnapshot::dsd_order_value,
                    AudioSettings::set_dsd_order,
                ),
            )
            .description(
                "How steeply noise is pushed above hearing. Higher takes more CPU and is not \
                 always quieter.",
            ),
            SettingItem::new(
                "Noise gain",
                dropdown(
                    audio,
                    settings.dsd_noise_gain_options(),
                    AudioSnapshot::dsd_noise_gain_value,
                    AudioSettings::set_dsd_noise_gain,
                ),
            )
            .description(
                "How hard noise is pushed above hearing. Higher lowers audible noise, but above \
                 1.40 loud music can stop playback unless the Limiter is at 0 dBTP or lower.",
            ),
            SettingItem::new(
                "Noise band",
                dropdown(
                    audio,
                    settings.dsd_noise_band_options(),
                    AudioSnapshot::dsd_noise_band_value,
                    AudioSettings::set_dsd_noise_band,
                ),
            )
            .description(
                "The range kept quiet, with the noise moved above it. Widening it to 48 kHz adds \
                 noise below 20 kHz, too much at rates under DSD256.",
            )
            .disabled(!settings.is_dsd_noise_band_widenable()),
            SettingItem::new("Optimized zeros", zeros).description(
                "Spreads the quietest points across that range instead of stacking them at its \
                 low end, which lowers the noise overall.",
            ),
            SettingItem::new(
                "Dither",
                dropdown(
                    audio,
                    settings.dsd_dither_options(),
                    AudioSnapshot::dsd_dither_value,
                    AudioSettings::set_dsd_dither,
                ),
            )
            .description(
                "Adds faint random noise, which can break up tones the modulator makes in very \
                 quiet passages.",
            ),
            SettingItem::new(
                "Input level",
                dropdown(
                    audio,
                    settings.dsd_input_level_options(),
                    AudioSnapshot::dsd_input_level_value,
                    AudioSettings::set_dsd_input_level,
                ),
            )
            .description(
                "Changes the song's level before conversion. Lower it if loud songs stop playback.",
            ),
            SettingItem::new(
                "Limiter",
                dropdown(
                    audio,
                    settings.dsd_limiter_options(),
                    AudioSnapshot::dsd_limiter_value,
                    AudioSettings::set_dsd_limiter,
                ),
            )
            .description(
                "Turns down peaks above this level, measured in true peak (dBTP), so loud songs \
                 don't overload the modulator. With Off, they can stop playback.",
            ),
        ]
        .into_iter()
        .map(|item| item.keywords(["dsd"])),
    )
}
