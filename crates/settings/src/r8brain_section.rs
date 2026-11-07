use config::{R8brainFilter, R8brainPerformance, R8brainQuality};
use gpui_kit::component::{
    radio::{Radio, RadioGroup},
    setting::{SettingField, SettingGroup, SettingItem},
};
use gpui_kit::{App, AppContext as _, Axis, Context, Entity, SharedString, Window};

use crate::audio_settings::AudioSettings;
use crate::r8brain_number_field::{R8brainNumber, R8brainNumberField};
use crate::r8brain_settings::{PHASES, R8BRAIN_QUALITIES};

pub(crate) struct R8brainFields {
    attenuation: Entity<R8brainNumberField>,
    transition_band: Entity<R8brainNumberField>,
    filter_fractions: Entity<R8brainNumberField>,
    max_input_frames: Entity<R8brainNumberField>,
    filter_cache_size: Entity<R8brainNumberField>,
    filter_bank_cache_size: Entity<R8brainNumberField>,
}

impl R8brainFields {
    pub(crate) fn new<T>(
        audio: &Entity<AudioSettings>,
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> Self {
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
pub(crate) fn r8brain_groups(
    audio: &Entity<AudioSettings>,
    fields: &R8brainFields,
    cx: &App,
) -> Vec<SettingGroup> {
    let settings = audio.read(cx);
    if !settings.uses_r8brain() || settings.is_dsd_on() {
        return Vec::new();
    }
    let filter = [
        SettingItem::new("Bit depth", bit_depth_field(audio, cx))
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
        SettingItem::new(
            "Whole-step filters",
            filter_switch(
                audio,
                |filter| filter.whole_stepping,
                |filter, is_on| filter.whole_stepping = is_on,
            ),
        )
        .description(
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
            move |cx| field.read(cx).is_resettable(cx)
        },
        {
            let field = field.clone();
            move |window, cx| field.update(cx, |field, cx| field.reset(window, cx))
        },
    );
    SettingItem::new(title, number)
}

/// For the Resampling group, in the place of Rubato's quality
pub(crate) fn r8brain_quality_item(audio: &Entity<AudioSettings>) -> SettingItem {
    SettingItem::new("Quality", quality_field(audio))
        .layout(Axis::Vertical)
        .description("Higher keeps more treble and removes more unwanted sound, with more delay.")
}

fn quality_field(audio: &Entity<AudioSettings>) -> SettingField<SharedString> {
    let reset_check = audio.clone();
    let reset = audio.clone();
    let audio = audio.clone();
    SettingField::render(move |_, _, cx| {
        let selected = audio.read(cx).r8brain_quality_ix();
        let is_dsd_on = audio.read(cx).is_dsd_on();
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
            .on_click(move |ix, _, cx| {
                audio.update(cx, |audio, cx| audio.select_r8brain_quality(*ix, cx));
            })
    })
    .on_reset(
        move |cx| reset_check.read(cx).r8brain_parameters().quality != R8brainQuality::default(),
        move |_, cx| {
            reset.update(cx, |audio, cx| {
                audio.change_r8brain(
                    |parameters| parameters.quality = R8brainQuality::default(),
                    cx,
                );
            });
        },
    )
}

fn bit_depth_field(audio: &Entity<AudioSettings>, cx: &App) -> SettingField<SharedString> {
    SettingField::scrollable_dropdown(
        audio.read(cx).r8brain_bit_depth_options(),
        {
            let audio = audio.clone();
            move |cx| audio.read(cx).r8brain_bit_depth_value()
        },
        {
            let audio = audio.clone();
            move |chosen: SharedString, cx| {
                audio.update(cx, |audio, cx| audio.set_r8brain_bit_depth(&chosen, cx));
            }
        },
    )
}

fn phase_field(audio: &Entity<AudioSettings>) -> SettingField<SharedString> {
    let audio = audio.clone();
    let reset_check = audio.clone();
    let reset = audio.clone();
    SettingField::render(move |_, _, cx| {
        let selected = audio.read(cx).r8brain_phase_ix();
        let audio = audio.clone();
        RadioGroup::horizontal("r8brain-phase")
            .selected_index(Some(selected))
            .children(
                PHASES
                    .iter()
                    .enumerate()
                    .map(|(ix, label)| Radio::new(("r8brain-phase", ix)).label(*label)),
            )
            .on_click(move |ix, _, cx| {
                audio.update(cx, |audio, cx| audio.select_r8brain_phase(*ix, cx));
            })
    })
    .on_reset(
        move |cx| reset_check.read(cx).r8brain_filter().phase != default_filter().phase,
        move |_, cx| {
            reset.update(cx, |audio, cx| {
                audio.change_r8brain_filter(|filter| filter.phase = default_filter().phase, cx);
            });
        },
    )
}

fn filter_switch(
    audio: &Entity<AudioSettings>,
    get: fn(&R8brainFilter) -> bool,
    set: fn(&mut R8brainFilter, bool),
) -> SettingField<bool> {
    let default = get(&default_filter());
    let reset_check = audio.clone();
    let reset = audio.clone();
    SettingField::switch(
        {
            let audio = audio.clone();
            move |cx| get(&audio.read(cx).r8brain_filter())
        },
        {
            let audio = audio.clone();
            move |is_on, cx| {
                audio.update(cx, |audio, cx| {
                    audio.change_r8brain_filter(|filter| set(filter, is_on), cx);
                });
            }
        },
    )
    .on_reset(
        move |cx| get(&reset_check.read(cx).r8brain_filter()) != default,
        move |_, cx| {
            reset.update(cx, |audio, cx| {
                audio.change_r8brain_filter(|filter| set(filter, default), cx);
            });
        },
    )
}

fn extended_fft_field(audio: &Entity<AudioSettings>) -> SettingField<bool> {
    let default = R8brainPerformance::default().extended_fft;
    let reset_check = audio.clone();
    let reset = audio.clone();
    SettingField::switch(
        {
            let audio = audio.clone();
            move |cx| audio.read(cx).r8brain_parameters().performance.extended_fft
        },
        {
            let audio = audio.clone();
            move |is_on, cx| {
                audio.update(cx, |audio, cx| {
                    audio.change_r8brain(
                        |parameters| parameters.performance.extended_fft = is_on,
                        cx,
                    );
                });
            }
        },
    )
    .on_reset(
        move |cx| {
            reset_check
                .read(cx)
                .r8brain_parameters()
                .performance
                .extended_fft
                != default
        },
        move |_, cx| {
            reset.update(cx, |audio, cx| {
                audio.change_r8brain(
                    |parameters| parameters.performance.extended_fft = default,
                    cx,
                );
            });
        },
    )
}

fn default_filter() -> R8brainFilter {
    R8brainQuality::default().filter()
}
