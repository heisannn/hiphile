//! The Audio page's dropdowns, whose choices the settings give as values and labels

use gpui_kit::SharedString;
use gpui_kit::component::setting::SettingField;
use hiphile_core::settings::{AudioSettings, AudioSnapshot, Choice};

pub(crate) type Options = Vec<(SharedString, SharedString)>;

pub(crate) fn options(choices: Vec<Choice>) -> Options {
    choices
        .into_iter()
        .map(|choice| {
            (
                SharedString::from(choice.value().to_owned()),
                SharedString::from(choice.label().to_owned()),
            )
        })
        .collect()
}

pub(crate) fn dropdown(
    audio: &AudioSettings,
    choices: Vec<Choice>,
    value: fn(&AudioSnapshot) -> String,
    set: fn(&AudioSettings, &str),
) -> SettingField<SharedString> {
    SettingField::scrollable_dropdown(
        options(choices),
        {
            let audio = audio.clone();
            move |_| value(&audio.snapshot()).into()
        },
        {
            let audio = audio.clone();
            move |chosen: SharedString, _| set(&audio, &chosen)
        },
    )
}
