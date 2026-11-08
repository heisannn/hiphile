use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::{
    WindowExt as _,
    radio::{Radio, RadioGroup},
    setting::{SettingField, SettingGroup, SettingItem},
};
use gpui_kit::{App, SharedString, Window};
use hiphile_core::settings::{AudioSettings, AudioSnapshot};

use crate::audio_options::dropdown;

const FORMATS: [&str; 2] = ["PCM", "DSD (DoP)"];
const DSD_FORMAT_IX: usize = 1;
const BASE_RATES: [&str; 2] = ["44.1 kHz", "Match each song"];
const MATCHING_BASE_RATE_IX: usize = 1;

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
        let is_on = settings.is_dsd_on();
        let is_blocked = settings.dsd_obstacle().is_some();
        let audio = audio.clone();
        RadioGroup::horizontal("output-format")
            .selected_index(Some(usize::from(is_on)))
            .disabled(is_blocked)
            .children(
                FORMATS
                    .iter()
                    .enumerate()
                    .map(|(ix, label)| Radio::new(("output-format", ix)).label(*label)),
            )
            .on_click(move |ix, window, cx| {
                let is_dsd = *ix == DSD_FORMAT_IX;
                if is_dsd == is_on {
                    return;
                }
                if is_dsd {
                    confirm_dsd(&audio, window, cx);
                } else {
                    audio.turn_dsd_off();
                }
            })
    })
}

/// A device that cannot decode DoP plays it as noise at full level, whatever
/// the app's volume was, and nothing the device reports can tell beforehand.
fn confirm_dsd(audio: &AudioSettings, window: &mut Window, cx: &mut App) {
    let name = audio.snapshot().dsd_device_name();
    let title = SharedString::from(format!("Turn on DSD for “{name}”?"));
    let audio = audio.clone();
    window.open_alert_dialog(cx, move |alert, _, _| {
        let audio = audio.clone();
        alert
            .title(title.clone())
            .description(
                "The app's volume stops working: sound comes at full level, so turn the \
                 device down first. A device without DoP support plays loud noise.",
            )
            .ok_text("Turn on")
            .ok_variant(ButtonVariant::Danger)
            .show_cancel(true)
            .on_ok(move |_, _, _| {
                audio.turn_dsd_on();
                true
            })
    });
}

fn base_rate_field(audio: &AudioSettings) -> SettingField<SharedString> {
    let audio = audio.clone();
    SettingField::render(move |_, _, _| {
        let matches = audio.snapshot().matches_song_family();
        let audio = audio.clone();
        RadioGroup::horizontal("dsd-base-rate")
            .selected_index(Some(usize::from(matches)))
            .children(
                BASE_RATES
                    .iter()
                    .enumerate()
                    .map(|(ix, label)| Radio::new(("dsd-base-rate", ix)).label(*label)),
            )
            .on_click(move |ix, window, cx| {
                let wants_matching = *ix == MATCHING_BASE_RATE_IX;
                if wants_matching == matches {
                    return;
                }
                if wants_matching {
                    confirm_matching_base_rate(&audio, window, cx);
                } else {
                    audio.set_matches_song_family(false);
                }
            })
    })
}

/// A DAC can decode the standard DSD rates and still take the 48 kHz family's
/// for PCM, which a song already playing would turn into noise at once.
fn confirm_matching_base_rate(audio: &AudioSettings, window: &mut Window, cx: &mut App) {
    let audio = audio.clone();
    window.open_alert_dialog(cx, move |alert, _, _| {
        let audio = audio.clone();
        alert
            .title("Match each song's base rate?")
            .description(
                "Songs at 48 kHz and its multiples then play at DSD rates based on 48 kHz. \
                 A DAC that doesn't decode those plays loud noise, so turn it down first.",
            )
            .ok_text("Match")
            .ok_variant(ButtonVariant::Danger)
            .show_cancel(true)
            .on_ok(move |_, _, _| {
                audio.set_matches_song_family(true);
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
