use anti_webui::prelude::*;
use hiphile_core::settings::{
    AudioSettings, AudioSnapshot, Choice, MATCH_SONG, PcmSetting, RESAMPLERS, RUBATO_QUALITIES,
    SYSTEM_DEFAULT,
};

use crate::choice_field::{choice_field, choice_of};
use crate::controls::{radios_of, switch_of, when};
use crate::dsd_section::dsd_groups;
use crate::group::{Group, Row, When, page};
use crate::r8brain_section::{r8brain_groups, r8brain_quality_row};
use crate::soxr_section::{soxr_groups, soxr_quality_row};

pub(crate) fn audio_page(audio: &AudioSettings) -> Div {
    let shown = next_host::follow(audio.subscribe());
    let resampling_off =
        when(move || shown.with(|shown| shown.dsd_note(PcmSetting::Resampling).is_some()));
    let resamples = move |with: fn(&AudioSnapshot) -> bool| {
        move || shown.with(|shown| with(shown) && shown.dsd_note(PcmSetting::Resampling).is_none())
    };
    let r8brain = {
        let audio = audio.clone();
        show(resamples(AudioSnapshot::uses_r8brain), move || {
            page(r8brain_groups(&audio, shown))
        })
    };
    let soxr = {
        let audio = audio.clone();
        show(resamples(AudioSnapshot::uses_soxr), move || {
            page(soxr_groups(&audio, shown))
        })
    };
    let mut groups = vec![
        output_group(audio, shown).into_view(),
        play_and_pause_group(audio, shown).into_view(),
        between_songs_group(audio, shown).into_view(),
        resampling_group(audio, shown, resampling_off).into_view(),
        r8brain.into_view(),
        soxr.into_view(),
    ];
    groups.extend(dsd_groups(audio, shown));
    page(groups)
}

/// What a PCM setting says while DSD keeps it from applying, which also disables it.
fn dsd_note(shown: Signal<AudioSnapshot>, setting: PcmSetting) -> impl Fn() -> Option<String> {
    move || shown.with(|shown| shown.dsd_note(setting).map(str::to_owned))
}

fn held_by_dsd(shown: Signal<AudioSnapshot>, setting: PcmSetting) -> When {
    when(move || shown.with(|shown| shown.dsd_note(setting).is_some()))
}

/// `value`, or `fallback` where the choices have no such value any more.
fn chosen_or(choices: &[Choice], value: String, fallback: &str) -> String {
    if choices.iter().any(|choice| choice.value() == value) {
        value
    } else {
        fallback.to_owned()
    }
}

fn output_group(audio: &AudioSettings, shown: Signal<AudioSnapshot>) -> Div {
    let no_devices = when(move || !shown.with(AudioSnapshot::has_devices));
    let device = {
        let audio = audio.clone();
        let off = no_devices.clone();
        Row::new("Output device", move || {
            let audio = audio.clone();
            choice_field(
                "Output device",
                move || shown.with(AudioSnapshot::device_options),
                move || {
                    shown.with(|shown| {
                        chosen_or(
                            &shown.device_options(),
                            shown.device_value(),
                            SYSTEM_DEFAULT,
                        )
                    })
                },
                move |value| audio.set_device(value),
                off.clone(),
            )
        })
        .disabled_when(move || no_devices())
    };
    let rate = {
        let audio = audio.clone();
        let off = held_by_dsd(shown, PcmSetting::SampleRate);
        let faded = off.clone();
        Row::new("Sample rate", move || {
            let audio = audio.clone();
            choice_field(
                "Sample rate",
                move || shown.with(AudioSnapshot::rate_options),
                move || {
                    shown.with(|shown| {
                        chosen_or(&shown.rate_options(), shown.rate_value(), MATCH_SONG)
                    })
                },
                move |value| audio.set_rate(value),
                off.clone(),
            )
        })
        .supporting(dsd_note(shown, PcmSetting::SampleRate))
        .disabled_when(move || faded())
    };
    let exclusive = AudioSettings::is_exclusive_offered().then(|| {
        let audio = audio.clone();
        let off = held_by_dsd(shown, PcmSetting::ExclusiveMode);
        let faded = off.clone();
        Row::new("Exclusive mode", move || {
            switch_of(
                "Exclusive mode",
                &audio,
                shown,
                AudioSnapshot::is_exclusive_on,
                AudioSettings::set_exclusive,
                off.clone(),
            )
        })
        .supporting(dsd_note(shown, PcmSetting::ExclusiveMode))
        .disabled_when(move || faded())
    });
    Group::new()
        .row(device)
        .row(rate)
        .rows(exclusive)
        .errors(move || shown.with(|shown| shown.errors().map(str::to_owned).collect()))
        .view()
}

fn play_and_pause_group(audio: &AudioSettings, shown: Signal<AudioSnapshot>) -> Div {
    let audio = audio.clone();
    let off = held_by_dsd(shown, PcmSetting::PlayPauseFade);
    let faded = off.clone();
    let fade = Row::new("Fade", move || {
        switch_of(
            "Fade",
            &audio,
            shown,
            AudioSnapshot::is_play_pause_fade_on,
            AudioSettings::set_play_pause_fade,
            off.clone(),
        )
    })
    .supporting(dsd_note(shown, PcmSetting::PlayPauseFade))
    .disabled_when(move || faded());
    Group::titled("Play and pause").row(fade).view()
}

fn between_songs_group(audio: &AudioSettings, shown: Signal<AudioSnapshot>) -> Div {
    let trim = {
        let audio = audio.clone();
        let off = held_by_dsd(shown, PcmSetting::TrimSilence);
        let faded = off.clone();
        let note = dsd_note(shown, PcmSetting::TrimSilence);
        Row::new("Trim silence", move || {
            switch_of(
                "Trim silence",
                &audio,
                shown,
                AudioSnapshot::is_trim_silence_on,
                AudioSettings::set_trim_silence,
                off.clone(),
            )
        })
        .supporting(move || {
            Some(note().unwrap_or_else(|| {
                "Silence longer than 10 seconds inside a song goes too.".to_owned()
            }))
        })
        .disabled_when(move || faded())
    };
    let crossfade = {
        let audio = audio.clone();
        let off = held_by_dsd(shown, PcmSetting::Crossfade);
        let faded = off.clone();
        Row::new("Crossfade", move || {
            let audio = audio.clone();
            choice_field(
                "Crossfade",
                AudioSettings::crossfade_options,
                move || shown.with(AudioSnapshot::crossfade_value),
                move |value| audio.set_crossfade(value),
                off.clone(),
            )
        })
        .supporting(move || shown.with(|shown| shown.crossfade_note().map(str::to_owned)))
        .disabled_when(move || faded())
    };
    Group::titled("Between songs")
        .rows([trim, crossfade])
        .view()
}

fn resampling_group(audio: &AudioSettings, shown: Signal<AudioSnapshot>, off: When) -> Div {
    let resampler = {
        let audio = audio.clone();
        let off = off.clone();
        Row::new("Resampler", move || {
            radios_of(
                "Resampler",
                &RESAMPLERS,
                &audio,
                shown,
                AudioSnapshot::resampler_ix,
                AudioSettings::select_resampler,
                off.clone(),
            )
        })
    };
    let uses = move |with: fn(&AudioSnapshot) -> bool| move || shown.with(with);
    let rubato = move || shown.with(|shown| !shown.uses_r8brain() && !shown.uses_soxr());
    let quality = {
        let audio = audio.clone();
        let off = off.clone();
        Row::new("Quality", move || {
            radios_of(
                "Quality",
                &RUBATO_QUALITIES,
                &audio,
                shown,
                AudioSnapshot::rubato_quality_ix,
                AudioSettings::select_rubato_quality,
                off.clone(),
            )
        })
        .visible_when(rubato)
    };
    let fft_size = {
        let audio = audio.clone();
        let off = off.clone();
        Row::new("FFT size", move || {
            choice_of(
                "FFT size",
                &audio,
                shown,
                AudioSnapshot::fft_size_options,
                AudioSnapshot::fft_size_value,
                AudioSettings::set_fft_size,
                off.clone(),
            )
        })
        .visible_when(rubato)
    };
    let window = {
        let audio = audio.clone();
        let off = off.clone();
        Row::new("Window", move || {
            choice_of(
                "Window",
                &audio,
                shown,
                AudioSnapshot::window_options,
                AudioSnapshot::window_value,
                AudioSettings::set_window,
                off.clone(),
            )
        })
        .visible_when(rubato)
    };
    let rows = [
        resampler,
        r8brain_quality_row(audio, shown, off.clone())
            .visible_when(uses(AudioSnapshot::uses_r8brain)),
        soxr_quality_row(audio, shown, off.clone()).visible_when(uses(AudioSnapshot::uses_soxr)),
        quality,
        fft_size,
        window,
    ]
    .map(|row| {
        let off = off.clone();
        row.disabled_when(move || off())
    });
    Group::titled("Resampling")
        .note(move || shown.with(|shown| shown.dsd_note(PcmSetting::Resampling).map(str::to_owned)))
        .rows(rows)
        .view()
}

#[cfg(test)]
mod tests;
