use config::{SoxrFilter, SoxrPerformance, SoxrQuality};
use gpui_kit::component::{
    radio::{Radio, RadioGroup},
    setting::{SettingField, SettingGroup, SettingItem},
};
use gpui_kit::{Axis, SharedString};
use hiphile_core::settings::{
    AudioSettings, AudioSnapshot, Choice, SOXR_INTERPOLATIONS, SOXR_PHASES, SOXR_QUALITIES,
};

use crate::audio_options::dropdown;

/// The Audio page's soxr groups, which stay out of the way unless it is what resamples.
pub(crate) fn soxr_groups(audio: &AudioSettings) -> Vec<SettingGroup> {
    let settings = audio.snapshot();
    if !settings.uses_soxr() || settings.is_dsd_on() {
        return Vec::new();
    }
    let filter = [
        SettingItem::new(
            "Precision",
            dropdown(
                audio,
                settings.soxr_precision_options(),
                AudioSnapshot::soxr_precision_value,
                AudioSettings::set_soxr_precision,
            ),
        )
        .description(
            "How far below the loudest sound unwanted sound stays, about 6 dB a bit. Higher takes \
             a longer filter.",
        ),
        SettingItem::new(
            "Pass-band end",
            dropdown(
                audio,
                settings.soxr_passband_end_options(),
                AudioSnapshot::soxr_passband_end_value,
                AudioSettings::set_soxr_passband_end,
            ),
        )
        .description(
            "Where the filter starts removing highs, as a fraction of the top frequency. Higher \
             keeps more treble and takes a longer filter.",
        ),
        SettingItem::new("Phase", phase_field(audio))
            .layout(Axis::Vertical)
            .description(
                "Minimum moves the ringing after each sound, but moves frequencies in time by \
                 different amounts. Intermediate goes halfway.",
            ),
        SettingItem::new(
            "Double precision",
            filter_switch(
                audio,
                |filter| filter.double_precision,
                |filter, is_on| filter.double_precision = is_on,
            ),
        )
        .description(
            "Computes in 64-bit floating point at 20 bits or less too, which rounds less and \
             takes longer. Higher precisions always do.",
        ),
    ];
    let performance = [
        SettingItem::new("Coefficient table", coef_table_field(audio, &settings)).description(
            "The most memory for the filter's steps between samples. With less, it works out \
             those it doesn't keep.",
        ),
        SettingItem::new("Coefficient interpolation", interpolation_field(audio))
            .layout(Axis::Vertical)
            .description(
                "How the filter works out the steps it doesn't keep. Linear takes more memory and \
                 Cubic more computing. Auto keeps every step a ratio of small whole numbers takes, \
                 if they fit.",
            ),
        SettingItem::new(
            "Smallest DFT",
            performance_dropdown(
                audio,
                settings.soxr_min_dft_options(),
                AudioSnapshot::soxr_min_dft_value,
                AudioSettings::set_soxr_min_dft,
                |performance| performance.min_dft_size,
                |performance, size| performance.min_dft_size = size,
            ),
        )
        .description(
            "The shortest block of the filter's FFT. Longer can run faster and adds delay.",
        ),
        SettingItem::new(
            "Largest DFT",
            performance_dropdown(
                audio,
                settings.soxr_large_dft_options(),
                AudioSnapshot::soxr_large_dft_value,
                AudioSettings::set_soxr_large_dft,
                |performance| performance.large_dft_size,
                |performance, size| performance.large_dft_size = size,
            ),
        )
        .description(
            "How long the FFT block may grow to run faster. A filter that needs a longer one \
             still gets it.",
        ),
    ];
    vec![
        SettingGroup::new()
            .title("soxr filter")
            .items(filter.into_iter().map(with_keywords)),
        SettingGroup::new()
            .title("soxr performance")
            .items(performance.into_iter().map(with_keywords)),
    ]
}

fn with_keywords(item: SettingItem) -> SettingItem {
    item.keywords(["resampling", "soxr", "libsoxr"])
}

/// For the Resampling group, in the place of Rubato's quality
pub(crate) fn soxr_quality_item(audio: &AudioSettings) -> SettingItem {
    SettingItem::new("Quality", quality_field(audio))
        .layout(Axis::Vertical)
        .description(
            "LQ and MQ remove the same amount of unwanted sound, and HQ and VHQ more. LQ also \
             lowers the highs, from about 18 kHz at 44.1 kHz and 19.5 kHz at 48 kHz.",
        )
}

fn quality_field(audio: &AudioSettings) -> SettingField<SharedString> {
    let reset_check = audio.clone();
    let reset = audio.clone();
    let audio = audio.clone();
    SettingField::render(move |_, _, _| {
        let selected = audio.snapshot().soxr_quality_ix();
        let is_dsd_on = audio.snapshot().is_dsd_on();
        let audio = audio.clone();
        RadioGroup::horizontal("soxr-quality")
            .selected_index(Some(selected))
            .disabled(is_dsd_on)
            .children(
                SOXR_QUALITIES
                    .iter()
                    .enumerate()
                    .map(|(ix, label)| Radio::new(("soxr-quality", ix)).label(*label)),
            )
            .on_click(move |ix, _, _| {
                audio.select_soxr_quality(*ix);
            })
    })
    .on_reset(
        move |_| reset_check.snapshot().soxr_parameters().quality != SoxrQuality::default(),
        move |_, _| {
            reset.change_soxr(|parameters| parameters.quality = SoxrQuality::default());
        },
    )
}

fn phase_field(audio: &AudioSettings) -> SettingField<SharedString> {
    let audio = audio.clone();
    let reset_check = audio.clone();
    let reset = audio.clone();
    SettingField::render(move |_, _, _| {
        let selected = audio.snapshot().soxr_phase_ix();
        let audio = audio.clone();
        RadioGroup::horizontal("soxr-phase")
            .selected_index(Some(selected))
            .children(
                SOXR_PHASES
                    .iter()
                    .enumerate()
                    .map(|(ix, label)| Radio::new(("soxr-phase", ix)).label(*label)),
            )
            .on_click(move |ix, _, _| {
                audio.select_soxr_phase(*ix);
            })
    })
    .on_reset(
        move |_| reset_check.snapshot().soxr_filter().phase != default_filter().phase,
        move |_, _| {
            reset.change_soxr_filter(|filter| filter.phase = default_filter().phase);
        },
    )
}

fn filter_switch(
    audio: &AudioSettings,
    get: fn(&SoxrFilter) -> bool,
    set: fn(&mut SoxrFilter, bool),
) -> SettingField<bool> {
    let default = get(&default_filter());
    let reset_check = audio.clone();
    let reset = audio.clone();
    SettingField::switch(
        {
            let audio = audio.clone();
            move |_| get(&audio.snapshot().soxr_filter())
        },
        {
            let audio = audio.clone();
            move |is_on, _| {
                audio.change_soxr_filter(|filter| set(filter, is_on));
            }
        },
    )
    .on_reset(
        move |_| get(&reset_check.snapshot().soxr_filter()) != default,
        move |_, _| {
            reset.change_soxr_filter(|filter| set(filter, default));
        },
    )
}

fn coef_table_field(audio: &AudioSettings, settings: &AudioSnapshot) -> SettingField<SharedString> {
    performance_dropdown(
        audio,
        settings.soxr_coef_table_options(),
        AudioSnapshot::soxr_coef_table_value,
        AudioSettings::set_soxr_coef_table,
        |performance| performance.coef_table_kb,
        |performance, kb| performance.coef_table_kb = kb,
    )
}

fn interpolation_field(audio: &AudioSettings) -> SettingField<SharedString> {
    let audio = audio.clone();
    let reset_check = audio.clone();
    let reset = audio.clone();
    let default = SoxrPerformance::default().coef_interpolation;
    SettingField::render(move |_, _, _| {
        let selected = audio.snapshot().soxr_interpolation_ix();
        let audio = audio.clone();
        RadioGroup::horizontal("soxr-interpolation")
            .selected_index(Some(selected))
            .children(
                SOXR_INTERPOLATIONS
                    .iter()
                    .enumerate()
                    .map(|(ix, label)| Radio::new(("soxr-interpolation", ix)).label(*label)),
            )
            .on_click(move |ix, _, _| {
                audio.select_soxr_interpolation(*ix);
            })
    })
    .on_reset(
        move |_| performance(&reset_check.snapshot()).coef_interpolation != default,
        move |_, _| {
            reset.change_soxr(|parameters| parameters.performance.coef_interpolation = default);
        },
    )
}

/// A dropdown of one of the performance values, which resets to its default.
fn performance_dropdown(
    audio: &AudioSettings,
    choices: Vec<Choice>,
    value: fn(&AudioSnapshot) -> String,
    set: fn(&AudioSettings, &str),
    get: fn(&SoxrPerformance) -> u32,
    put: fn(&mut SoxrPerformance, u32),
) -> SettingField<SharedString> {
    let default = get(&SoxrPerformance::default());
    let reset_check = audio.clone();
    let reset = audio.clone();
    dropdown(audio, choices, value, set).on_reset(
        move |_| get(&performance(&reset_check.snapshot())) != default,
        move |_, _| {
            reset.change_soxr(|parameters| put(&mut parameters.performance, default));
        },
    )
}

fn performance(audio: &AudioSnapshot) -> SoxrPerformance {
    audio.soxr_parameters().performance
}

fn default_filter() -> SoxrFilter {
    SoxrQuality::default().filter()
}
