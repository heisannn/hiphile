use anti_webui::prelude::*;
use hiphile_core::settings::{
    AudioSettings, AudioSnapshot, R8BRAIN_PHASES, R8BRAIN_QUALITIES, R8brainNumber, R8brainSetting,
};

use crate::choice_field::choice_of;
use crate::controls::{never, radios_of, reset_button, resettable, switch_of};
use crate::group::{Group, Row, When};
use crate::number_field::r8brain_number_field;

/// The groups of the Audio page while r8brain resamples.
pub(crate) fn r8brain_groups(audio: &AudioSettings, shown: Signal<AudioSnapshot>) -> Vec<View> {
    let number = |number: R8brainNumber, headline: &'static str, supporting: &'static str| {
        let audio = audio.clone();
        Row::new(headline, move || {
            r8brain_number_field(&audio, shown, number, never())
        })
        .supporting_text(supporting)
    };
    let bit_depth = {
        let audio = audio.clone();
        Row::new("Bit depth", move || {
            choice_of(
                "Bit depth",
                &audio,
                shown,
                AudioSnapshot::r8brain_bit_depth_options,
                AudioSnapshot::r8brain_bit_depth_value,
                AudioSettings::set_r8brain_bit_depth,
                never(),
            )
        })
        .supporting_text("Sets the attenuation for audio of that depth.")
    };
    let phase = {
        let audio = audio.clone();
        Row::new("Phase", move || {
            reset(
                radios_of(
                    "Phase",
                    &R8BRAIN_PHASES,
                    &audio,
                    shown,
                    AudioSnapshot::r8brain_phase_ix,
                    AudioSettings::select_r8brain_phase,
                    never(),
                ),
                &audio,
                shown,
                R8brainSetting::Phase,
                "r8brain phase",
            )
        })
        .supporting_text(
            "Minimum adds far less delay, but moves frequencies in time by different amounts.",
        )
    };
    let whole_stepping = {
        let audio = audio.clone();
        Row::new("Whole-step filters", move || {
            reset(
                switch_of(
                    "Whole-step filters",
                    &audio,
                    shown,
                    |shown| shown.r8brain_filter().whole_stepping,
                    AudioSettings::set_r8brain_whole_stepping,
                    never(),
                ),
                &audio,
                shown,
                R8brainSetting::WholeStepping,
                "r8brain whole-step filters",
            )
        })
        .supporting_text(
            "Rates at a ratio of small whole numbers get a filter per position, which keeps time \
             exactly. Off interpolates as at any other ratio.",
        )
    };
    let extended_fft = {
        let audio = audio.clone();
        Row::new("Extended FFT", move || {
            reset(
                switch_of(
                    "Extended FFT",
                    &audio,
                    shown,
                    |shown| shown.r8brain_parameters().performance.extended_fft,
                    AudioSettings::set_r8brain_extended_fft,
                    never(),
                ),
                &audio,
                shown,
                R8brainSetting::ExtendedFft,
                "r8brain extended FFT",
            )
        })
        .supporting_text(
            "Doubles the filter's FFT block, which usually runs faster and adds delay.",
        )
    };
    let filter = [
        bit_depth,
        number(
            R8brainNumber::Attenuation,
            "Attenuation",
            "How far below the loudest sound unwanted sound stays. Higher takes a longer filter \
             and adds delay.",
        ),
        number(
            R8brainNumber::TransitionBand,
            "Transition band",
            "The share of the frequency range where the filter cuts off, ending at the top. Lower \
             keeps more treble and adds delay.",
        ),
        phase,
        whole_stepping,
        number(
            R8brainNumber::FilterFractions,
            "Filter positions",
            "How finely interpolation steps between samples. Empty chooses from the attenuation. \
             Whole-step filters don't use it.",
        ),
    ];
    let performance = [
        extended_fft,
        number(
            R8brainNumber::MaxInputFrames,
            "Input block",
            "How much of a song goes through at a time. Larger helps when lowering the rate, and \
             smaller saves memory when raising it a lot.",
        ),
        number(
            R8brainNumber::FilterCacheSize,
            "Filter cache",
            "Filters kept in memory for later songs.",
        ),
        number(
            R8brainNumber::FilterBankCacheSize,
            "Filter bank cache",
            "Whole-step filter banks kept in memory for later songs. They're large.",
        ),
    ];
    vec![
        Group::titled("r8brain filter")
            .rows(filter)
            .view()
            .into_view(),
        Group::titled("r8brain performance")
            .rows(performance)
            .view()
            .into_view(),
    ]
}

/// For the Resampling group, in the place of Rubato's quality.
pub(crate) fn r8brain_quality_row(
    audio: &AudioSettings,
    shown: Signal<AudioSnapshot>,
    disabled: When,
) -> Row {
    let audio = audio.clone();
    Row::new("Quality", move || {
        reset(
            radios_of(
                "Quality",
                &R8BRAIN_QUALITIES,
                &audio,
                shown,
                AudioSnapshot::r8brain_quality_ix,
                AudioSettings::select_r8brain_quality,
                disabled.clone(),
            ),
            &audio,
            shown,
            R8brainSetting::Quality,
            "r8brain quality",
        )
    })
    .supporting_text("Higher keeps more treble and removes more unwanted sound, with more delay.")
    .stacked()
}

fn reset(
    control: Div,
    audio: &AudioSettings,
    shown: Signal<AudioSnapshot>,
    setting: R8brainSetting,
    name: &'static str,
) -> Div {
    let audio = audio.clone();
    resettable(
        control,
        reset_button(
            name,
            move || shown.with(|shown| shown.is_r8brain_resettable(setting)),
            move || audio.reset_r8brain(setting),
        ),
    )
}
