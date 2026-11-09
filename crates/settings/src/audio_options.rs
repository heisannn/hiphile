//! The dropdowns of the Audio and DSP pages, whose choices the settings give as values and labels

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
    dropdown_of(audio, AudioSettings::snapshot, choices, value, set)
}

/// As [`dropdown`], for settings that `snapshot` reads.
pub(crate) fn dropdown_of<S: Clone + 'static, T: 'static>(
    settings: &S,
    snapshot: fn(&S) -> T,
    choices: Vec<Choice>,
    value: fn(&T) -> String,
    set: fn(&S, &str),
) -> SettingField<SharedString> {
    SettingField::scrollable_dropdown(
        options(choices),
        {
            let settings = settings.clone();
            move |_| value(&snapshot(&settings)).into()
        },
        {
            let settings = settings.clone();
            move |chosen: SharedString, _| set(&settings, &chosen)
        },
    )
}
