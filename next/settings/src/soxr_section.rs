use anti_webui::prelude::*;
use hiphile_core::settings::{
    AudioSettings, AudioSnapshot, Choice, SOXR_INTERPOLATIONS, SOXR_PHASES, SOXR_QUALITIES,
    SoxrSetting,
};

use crate::choice_field::choice_of;
use crate::controls::{never, radios_of, reset_button, resettable, switch_of};
use crate::group::{Group, Row, When};

type Options = fn(&AudioSnapshot) -> Vec<Choice>;
type Value = fn(&AudioSnapshot) -> String;
type Set = fn(&AudioSettings, &str);

/// The groups of the Audio page while soxr resamples.
pub(crate) fn soxr_groups(audio: &AudioSettings, shown: Signal<AudioSnapshot>) -> Vec<View> {
    let choice = |headline: &'static str,
                  options: Options,
                  value: Value,
                  set: Set,
                  setting: SoxrSetting,
                  supporting: &'static str| {
        let audio = audio.clone();
        Row::new(headline, move || {
            reset(
                choice_of(headline, &audio, shown, options, value, set, never()),
                &audio,
                shown,
                setting,
                format!("soxr {}", headline.to_lowercase()),
            )
        })
        .supporting_text(supporting)
    };
    let phase = {
        let audio = audio.clone();
        Row::new("Phase", move || {
            reset(
                radios_of(
                    "Phase",
                    &SOXR_PHASES,
                    &audio,
                    shown,
                    AudioSnapshot::soxr_phase_ix,
                    AudioSettings::select_soxr_phase,
                    never(),
                ),
                &audio,
                shown,
                SoxrSetting::Phase,
                "soxr phase",
            )
        })
        .supporting_text(
            "Minimum moves the ringing after each sound, but moves frequencies in time by \
             different amounts. Intermediate goes halfway.",
        )
        .stacked()
    };
    let double_precision = {
        let audio = audio.clone();
        Row::new("Double precision", move || {
            reset(
                switch_of(
                    "Double precision",
                    &audio,
                    shown,
                    |shown| shown.soxr_filter().double_precision,
                    AudioSettings::set_soxr_double_precision,
                    never(),
                ),
                &audio,
                shown,
                SoxrSetting::DoublePrecision,
                "soxr double precision",
            )
        })
        .supporting_text(
            "Computes in 64-bit floating point at 20 bits or less too, which rounds less and \
             takes longer. Higher precisions always do.",
        )
    };
    let interpolation = {
        let audio = audio.clone();
        Row::new("Coefficient interpolation", move || {
            reset(
                radios_of(
                    "Coefficient interpolation",
                    &SOXR_INTERPOLATIONS,
                    &audio,
                    shown,
                    AudioSnapshot::soxr_interpolation_ix,
                    AudioSettings::select_soxr_interpolation,
                    never(),
                ),
                &audio,
                shown,
                SoxrSetting::Interpolation,
                "soxr coefficient interpolation",
            )
        })
        .supporting_text(
            "How the filter works out the steps it doesn't keep. Linear takes more memory and \
             Cubic more computing. Auto keeps every step a ratio of small whole numbers takes, if \
             they fit.",
        )
        .stacked()
    };
    let filter = [
        choice(
            "Precision",
            AudioSnapshot::soxr_precision_options,
            AudioSnapshot::soxr_precision_value,
            AudioSettings::set_soxr_precision,
            SoxrSetting::Precision,
            "How far below the loudest sound unwanted sound stays, about 6 dB a bit. Higher takes \
             a longer filter.",
        ),
        choice(
            "Pass-band end",
            AudioSnapshot::soxr_passband_end_options,
            AudioSnapshot::soxr_passband_end_value,
            AudioSettings::set_soxr_passband_end,
            SoxrSetting::PassbandEnd,
            "Where the filter starts removing highs, as a fraction of the top frequency. Higher \
             keeps more treble and takes a longer filter.",
        ),
        phase,
        double_precision,
    ];
    let performance = [
        choice(
            "Coefficient table",
            AudioSnapshot::soxr_coef_table_options,
            AudioSnapshot::soxr_coef_table_value,
            AudioSettings::set_soxr_coef_table,
            SoxrSetting::CoefTable,
            "The most memory for the filter's steps between samples. With less, it works out \
             those it doesn't keep.",
        ),
        interpolation,
        choice(
            "Smallest DFT",
            AudioSnapshot::soxr_min_dft_options,
            AudioSnapshot::soxr_min_dft_value,
            AudioSettings::set_soxr_min_dft,
            SoxrSetting::MinDft,
            "The shortest block of the filter's FFT. Longer can run faster and adds delay.",
        ),
        choice(
            "Largest DFT",
            AudioSnapshot::soxr_large_dft_options,
            AudioSnapshot::soxr_large_dft_value,
            AudioSettings::set_soxr_large_dft,
            SoxrSetting::LargeDft,
            "How long the FFT block may grow to run faster. A filter that needs a longer one \
             still gets it.",
        ),
    ];
    vec![
        Group::titled("soxr filter").rows(filter).view().into_view(),
        Group::titled("soxr performance")
            .rows(performance)
            .view()
            .into_view(),
    ]
}

/// For the Resampling group, in the place of Rubato's quality.
pub(crate) fn soxr_quality_row(
    audio: &AudioSettings,
    shown: Signal<AudioSnapshot>,
    disabled: When,
) -> Row {
    let audio = audio.clone();
    Row::new("Quality", move || {
        reset(
            radios_of(
                "Quality",
                &SOXR_QUALITIES,
                &audio,
                shown,
                AudioSnapshot::soxr_quality_ix,
                AudioSettings::select_soxr_quality,
                disabled.clone(),
            ),
            &audio,
            shown,
            SoxrSetting::Quality,
            "soxr quality",
        )
    })
    .supporting_text(
        "LQ and MQ remove the same amount of unwanted sound, and HQ and VHQ more. LQ also lowers \
         the highs, from about 18 kHz at 44.1 kHz and 19.5 kHz at 48 kHz.",
    )
    .stacked()
}

fn reset(
    control: Div,
    audio: &AudioSettings,
    shown: Signal<AudioSnapshot>,
    setting: SoxrSetting,
    name: impl Into<String>,
) -> Div {
    let audio = audio.clone();
    resettable(
        control,
        reset_button(
            name,
            move || shown.with(|shown| shown.is_soxr_resettable(setting)),
            move || audio.reset_soxr(setting),
        ),
    )
}
