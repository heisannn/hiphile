use gpui_kit::component::{
    radio::{Radio, RadioGroup},
    setting::{SettingField, SettingGroup, SettingItem},
};
use gpui_kit::{AppContext as _, Axis, Context, Entity, SharedString, Window};
use hiphile_core::settings::{
    AudioSettings, AudioSnapshot, PcmSetting, R8BRAIN_PHASES, R8BRAIN_QUALITIES, R8brainNumber,
    R8brainSetting,
};

use crate::audio_options::dropdown;
use crate::r8brain_number_field::R8brainNumberField;

pub(crate) struct R8brainFields {
    attenuation: Entity<R8brainNumberField>,
    transition_band: Entity<R8brainNumberField>,
    filter_fractions: Entity<R8brainNumberField>,
    max_input_frames: Entity<R8brainNumberField>,
    filter_cache_size: Entity<R8brainNumberField>,
    filter_bank_cache_size: Entity<R8brainNumberField>,
}

impl R8brainFields {
    pub(crate) fn new<T>(audio: &AudioSettings, window: &mut Window, cx: &mut Context<T>) -> Self {
        let mut field = |number| {
            let audio = audio.clone();
            cx.new(|cx| R8brainNumberField::new(audio, number, window, cx))
        };
        Self {
            attenuation: field(R8brainNumber::Attenuation),
            transition_band: field(R8brainNumber::TransitionBand),
            filter_fractions: field(R8brainNumber::FilterFractions),
            max_input_frames: field(R8brainNumber::MaxInputFrames),
            filter_cache_size: field(R8brainNumber::FilterCacheSize),
            filter_bank_cache_size: field(R8brainNumber::FilterBankCacheSize),
        }
    }
}

/// The Audio page's r8brain groups, which stay out of the way unless it is what resamples.
pub(crate) fn r8brain_groups(audio: &AudioSettings, fields: &R8brainFields) -> Vec<SettingGroup> {
    let settings = audio.snapshot();
    if !settings.uses_r8brain() || settings.dsd_note(PcmSetting::Resampling).is_some() {
        return Vec::new();
    }
    let filter = [
        SettingItem::new("Bit depth", bit_depth_field(audio, &settings))
            .description("Sets the attenuation for audio of that depth."),
        number_item(&fields.attenuation, "Attenuation").description(
            "How far below the loudest sound unwanted sound stays. Higher takes a longer filter \
             and adds delay.",
        ),
        number_item(&fields.transition_band, "Transition band").description(
            "The share of the frequency range where the filter cuts off, ending at the top. \
             Lower keeps more treble and adds delay.",
        ),
        SettingItem::new("Phase", phase_field(audio)).description(
            "Minimum adds far less delay, but moves frequencies in time by different amounts.",
        ),
        SettingItem::new("Whole-step filters", whole_stepping_field(audio)).description(
            "Rates at a ratio of small whole numbers get a filter per position, which keeps time \
             exactly. Off interpolates as at any other ratio.",
        ),
        number_item(&fields.filter_fractions, "Filter positions").description(
            "How finely interpolation steps between samples. Empty chooses from the attenuation. \
             Whole-step filters don't use it.",
        ),
    ];
    let performance = [
        SettingItem::new("Extended FFT", extended_fft_field(audio)).description(
            "Doubles the filter's FFT block, which usually runs faster and adds delay.",
        ),
        number_item(&fields.max_input_frames, "Input block").description(
            "How much of a song goes through at a time. Larger helps when lowering the rate, and \
             smaller saves memory when raising it a lot.",
        ),
        number_item(&fields.filter_cache_size, "Filter cache")
            .description("Filters kept in memory for later songs."),
        number_item(&fields.filter_bank_cache_size, "Filter bank cache")
            .description("Whole-step filter banks kept in memory for later songs. They're large."),
    ];
    vec![
        SettingGroup::new()
            .title("r8brain filter")
            .items(filter.into_iter().map(with_keywords)),
        SettingGroup::new()
            .title("r8brain performance")
            .items(performance.into_iter().map(with_keywords)),
    ]
}

fn with_keywords(item: SettingItem) -> SettingItem {
    item.keywords(["resampling", "r8brain"])
}

fn number_item(field: &Entity<R8brainNumberField>, title: &'static str) -> SettingItem {
    let number = SettingField::render({
        let field = field.clone();
        move |_, _, _| field.clone()
    })
    .on_reset(
        {
            let field = field.clone();
            move |cx| field.read(cx).is_resettable()
        },
        {
            let field = field.clone();
            move |window, cx| field.update(cx, |field, cx| field.reset(window, cx))
        },
    );
    SettingItem::new(title, number)
}

/// For the Resampling group, in the place of Rubato's quality
pub(crate) fn r8brain_quality_item(audio: &AudioSettings) -> SettingItem {
    SettingItem::new("Quality", quality_field(audio))
        .layout(Axis::Vertical)
        .description("Higher keeps more treble and removes more unwanted sound, with more delay.")
}

fn quality_field(audio: &AudioSettings) -> SettingField<SharedString> {
    let field = SettingField::render({
        let audio = audio.clone();
        move |_, _, _| {
            let settings = audio.snapshot();
            let selected = settings.r8brain_quality_ix();
            let is_dsd_on = settings.dsd_note(PcmSetting::Resampling).is_some();
            let audio = audio.clone();
            RadioGroup::horizontal("r8brain-quality")
                .selected_index(Some(selected))
                .disabled(is_dsd_on)
                .children(
                    R8BRAIN_QUALITIES
                        .iter()
                        .enumerate()
                        .map(|(ix, label)| Radio::new(("r8brain-quality", ix)).label(*label)),
                )
                .on_click(move |ix, _, _| {
                    audio.select_r8brain_quality(*ix);
                })
        }
    });
    resettable(field, audio, R8brainSetting::Quality)
}

fn bit_depth_field(audio: &AudioSettings, settings: &AudioSnapshot) -> SettingField<SharedString> {
    dropdown(
        audio,
        settings.r8brain_bit_depth_options(),
        AudioSnapshot::r8brain_bit_depth_value,
        AudioSettings::set_r8brain_bit_depth,
    )
}

fn phase_field(audio: &AudioSettings) -> SettingField<SharedString> {
    let field = SettingField::render({
        let audio = audio.clone();
        move |_, _, _| {
            let selected = audio.snapshot().r8brain_phase_ix();
            let audio = audio.clone();
            RadioGroup::horizontal("r8brain-phase")
                .selected_index(Some(selected))
                .children(
                    R8BRAIN_PHASES
                        .iter()
                        .enumerate()
                        .map(|(ix, label)| Radio::new(("r8brain-phase", ix)).label(*label)),
                )
                .on_click(move |ix, _, _| {
                    audio.select_r8brain_phase(*ix);
                })
        }
    });
    resettable(field, audio, R8brainSetting::Phase)
}

fn whole_stepping_field(audio: &AudioSettings) -> SettingField<bool> {
    let field = SettingField::switch(
        {
            let audio = audio.clone();
            move |_| audio.snapshot().r8brain_filter().whole_stepping
        },
        {
            let audio = audio.clone();
            move |is_on, _| audio.set_r8brain_whole_stepping(is_on)
        },
    );
    resettable(field, audio, R8brainSetting::WholeStepping)
}

fn extended_fft_field(audio: &AudioSettings) -> SettingField<bool> {
    let field = SettingField::switch(
        {
            let audio = audio.clone();
            move |_| {
                audio
                    .snapshot()
                    .r8brain_parameters()
                    .performance
                    .extended_fft
            }
        },
        {
            let audio = audio.clone();
            move |is_on, _| audio.set_r8brain_extended_fft(is_on)
        },
    );
    resettable(field, audio, R8brainSetting::ExtendedFft)
}

fn resettable<T>(
    field: SettingField<T>,
    audio: &AudioSettings,
    setting: R8brainSetting,
) -> SettingField<T> {
    let (check, reset) = (audio.clone(), audio.clone());
    field.on_reset(
        move |_| check.snapshot().is_r8brain_resettable(setting),
        move |_, _| reset.reset_r8brain(setting),
    )
}
