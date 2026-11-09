use gpui_kit::component::{
    radio::{Radio, RadioGroup},
    setting::{SettingField, SettingGroup, SettingItem},
};
use gpui_kit::{Axis, SharedString};
use hiphile_core::settings::{
    AudioSettings, AudioSnapshot, Choice, PcmSetting, SOXR_INTERPOLATIONS, SOXR_PHASES,
    SOXR_QUALITIES, SoxrSetting,
};

use crate::audio_options::dropdown;

/// The Audio page's soxr groups, which stay out of the way unless it is what resamples.
pub(crate) fn soxr_groups(audio: &AudioSettings) -> Vec<SettingGroup> {
    let settings = audio.snapshot();
    if !settings.uses_soxr() || settings.dsd_note(PcmSetting::Resampling).is_some() {
        return Vec::new();
    }
    let filter = [
        SettingItem::new(
            "Precision",
            soxr_dropdown(
                audio,
                settings.soxr_precision_options(),
                AudioSnapshot::soxr_precision_value,
                AudioSettings::set_soxr_precision,
                SoxrSetting::Precision,
            ),
        )
        .description(
            "How far below the loudest sound unwanted sound stays, about 6 dB a bit. Higher takes \
             a longer filter.",
        ),
        SettingItem::new(
            "Pass-band end",
            soxr_dropdown(
                audio,
                settings.soxr_passband_end_options(),
                AudioSnapshot::soxr_passband_end_value,
                AudioSettings::set_soxr_passband_end,
                SoxrSetting::PassbandEnd,
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
        SettingItem::new("Double precision", double_precision_field(audio)).description(
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
            soxr_dropdown(
                audio,
                settings.soxr_min_dft_options(),
                AudioSnapshot::soxr_min_dft_value,
                AudioSettings::set_soxr_min_dft,
                SoxrSetting::MinDft,
            ),
        )
        .description(
            "The shortest block of the filter's FFT. Longer can run faster and adds delay.",
        ),
        SettingItem::new(
            "Largest DFT",
            soxr_dropdown(
                audio,
                settings.soxr_large_dft_options(),
                AudioSnapshot::soxr_large_dft_value,
                AudioSettings::set_soxr_large_dft,
                SoxrSetting::LargeDft,
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
    let field = SettingField::render({
        let audio = audio.clone();
        move |_, _, _| {
            let settings = audio.snapshot();
            let selected = settings.soxr_quality_ix();
            let is_dsd_on = settings.dsd_note(PcmSetting::Resampling).is_some();
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
        }
    });
    resettable(field, audio, SoxrSetting::Quality)
}

fn phase_field(audio: &AudioSettings) -> SettingField<SharedString> {
    let field = SettingField::render({
        let audio = audio.clone();
        move |_, _, _| {
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
        }
    });
    resettable(field, audio, SoxrSetting::Phase)
}

fn double_precision_field(audio: &AudioSettings) -> SettingField<bool> {
    let field = SettingField::switch(
        {
            let audio = audio.clone();
            move |_| audio.snapshot().soxr_filter().double_precision
        },
        {
            let audio = audio.clone();
            move |is_on, _| audio.set_soxr_double_precision(is_on)
        },
    );
    resettable(field, audio, SoxrSetting::DoublePrecision)
}

fn coef_table_field(audio: &AudioSettings, settings: &AudioSnapshot) -> SettingField<SharedString> {
    soxr_dropdown(
        audio,
        settings.soxr_coef_table_options(),
        AudioSnapshot::soxr_coef_table_value,
        AudioSettings::set_soxr_coef_table,
        SoxrSetting::CoefTable,
    )
}

fn interpolation_field(audio: &AudioSettings) -> SettingField<SharedString> {
    let field = SettingField::render({
        let audio = audio.clone();
        move |_, _, _| {
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
        }
    });
    resettable(field, audio, SoxrSetting::Interpolation)
}

fn soxr_dropdown(
    audio: &AudioSettings,
    choices: Vec<Choice>,
    value: fn(&AudioSnapshot) -> String,
    set: fn(&AudioSettings, &str),
    setting: SoxrSetting,
) -> SettingField<SharedString> {
    resettable(dropdown(audio, choices, value, set), audio, setting)
}

fn resettable<T>(
    field: SettingField<T>,
    audio: &AudioSettings,
    setting: SoxrSetting,
) -> SettingField<T> {
    let (check, reset) = (audio.clone(), audio.clone());
    field.on_reset(
        move |_| check.snapshot().is_soxr_resettable(setting),
        move |_, _| reset.reset_soxr(setting),
    )
}
