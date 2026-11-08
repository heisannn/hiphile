//! The choices of the Audio page's dropdowns, each a value and its label

use gpui_kit::component::setting::SettingField;
use gpui_kit::{Context, Entity, SharedString};

use crate::audio_settings::AudioSettings;

pub(crate) type Options = Vec<(SharedString, SharedString)>;

pub(crate) fn same_labels(values: impl Iterator<Item = String>) -> Options {
    values
        .map(|value| {
            let value = SharedString::from(value);
            (value.clone(), value)
        })
        .collect()
}

pub(crate) fn number_value(number: f64) -> SharedString {
    number.to_string().into()
}

/// The values, and `current` too if the file holds one that is not among them,
/// so that the choice stays visible.
pub(crate) fn numbers(values: &[f64], current: f64, label: impl Fn(f64) -> String) -> Options {
    let mut options: Options = values
        .iter()
        .map(|number| (number_value(*number), label(*number).into()))
        .collect();
    if !values.contains(&current) {
        options.push((number_value(current), label(current).into()));
    }
    options
}

pub(crate) fn dropdown(
    audio: &Entity<AudioSettings>,
    options: Options,
    value: fn(&AudioSettings) -> SharedString,
    set: fn(&mut AudioSettings, &str, &mut Context<AudioSettings>),
) -> SettingField<SharedString> {
    SettingField::scrollable_dropdown(
        options,
        {
            let audio = audio.clone();
            move |cx| value(audio.read(cx))
        },
        {
            let audio = audio.clone();
            move |chosen: SharedString, cx| {
                audio.update(cx, |audio, cx| set(audio, &chosen, cx));
            }
        },
    )
}

#[cfg(test)]
mod tests;
