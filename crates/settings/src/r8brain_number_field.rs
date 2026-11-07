use std::time::Duration;

use config::{
    R8BRAIN_ATTENUATIONS, R8BRAIN_CACHE_SIZES, R8BRAIN_FILTER_FRACTIONS, R8BRAIN_MAX_INPUT_FRAMES,
    R8BRAIN_TRANSITION_BANDS, R8brainParameters, R8brainQuality,
};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, Render, Subscription, Task, Window,
};

use crate::audio_settings::AudioSettings;
use crate::number_field::{NumberField, NumberRange};

/// The field reports every keystroke, and each change restarts the resampling of what is playing.
const SETTLE: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum R8brainNumber {
    Attenuation,
    TransitionBand,
    FilterFractions,
    MaxInputFrames,
    FilterCacheSize,
    FilterBankCacheSize,
}

impl R8brainNumber {
    fn range(self) -> NumberRange {
        let whole = |range: std::ops::RangeInclusive<u32>, step| NumberRange {
            min: f64::from(*range.start()),
            max: f64::from(*range.end()),
            step,
            fraction: Some(0),
        };
        let decimal = |range: std::ops::RangeInclusive<f64>, step| NumberRange {
            min: *range.start(),
            max: *range.end(),
            step,
            fraction: None,
        };
        match self {
            Self::Attenuation => decimal(R8BRAIN_ATTENUATIONS, 1.0),
            Self::TransitionBand => decimal(R8BRAIN_TRANSITION_BANDS, 0.5),
            Self::FilterFractions => whole(R8BRAIN_FILTER_FRACTIONS, 256.0),
            Self::MaxInputFrames => whole(R8BRAIN_MAX_INPUT_FRAMES, 256.0),
            Self::FilterCacheSize | Self::FilterBankCacheSize => whole(R8BRAIN_CACHE_SIZES, 1.0),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Attenuation => "r8brain attenuation",
            Self::TransitionBand => "r8brain transition band",
            Self::FilterFractions => "r8brain filter positions",
            Self::MaxInputFrames => "r8brain input block",
            Self::FilterCacheSize => "r8brain filter cache",
            Self::FilterBankCacheSize => "r8brain filter bank cache",
        }
    }

    fn unit(self) -> &'static str {
        match self {
            Self::Attenuation => "dB",
            Self::TransitionBand => "%",
            Self::MaxInputFrames => "frames",
            Self::FilterFractions | Self::FilterCacheSize | Self::FilterBankCacheSize => "",
        }
    }

    /// Choosing a preset brings these back, and a reset to the default would leave the preset
    fn differs_between_presets(self) -> bool {
        matches!(self, Self::Attenuation | Self::TransitionBand)
    }

    /// `None` only where an empty field means something.
    fn value(self, parameters: &R8brainParameters) -> Option<f64> {
        let filter = parameters.quality.filter();
        let performance = parameters.performance;
        match self {
            Self::Attenuation => Some(filter.attenuation),
            Self::TransitionBand => Some(filter.transition_band),
            Self::FilterFractions => filter.filter_fractions.map(f64::from),
            Self::MaxInputFrames => Some(f64::from(performance.max_input_frames)),
            Self::FilterCacheSize => Some(f64::from(performance.filter_cache_size)),
            Self::FilterBankCacheSize => Some(f64::from(performance.filter_bank_cache_size)),
        }
    }

    fn set(self, parameters: &mut R8brainParameters, value: Option<f64>) {
        let whole = |value: f64| value.round() as u32;
        let mut filter = parameters.quality.filter();
        let performance = &mut parameters.performance;
        match (self, value) {
            (Self::FilterFractions, value) => filter.filter_fractions = value.map(whole),
            (Self::Attenuation, Some(value)) => filter.attenuation = value,
            (Self::TransitionBand, Some(value)) => filter.transition_band = value,
            (Self::MaxInputFrames, Some(value)) => performance.max_input_frames = whole(value),
            (Self::FilterCacheSize, Some(value)) => performance.filter_cache_size = whole(value),
            (Self::FilterBankCacheSize, Some(value)) => {
                performance.filter_bank_cache_size = whole(value);
            }
            (_, None) => {}
        }
        if filter != parameters.quality.filter() {
            parameters.quality = R8brainQuality::Custom(filter);
        }
    }

    fn text(self, parameters: &R8brainParameters) -> String {
        self.value(parameters)
            .map_or_else(String::new, |value| value.to_string())
    }

    /// The value that `text` stands for, or `None` if it stands for no value yet.
    fn parse(self, text: &str) -> Option<Option<f64>> {
        if text.is_empty() && self == Self::FilterFractions {
            return Some(None);
        }
        self.range().parse(text).map(Some)
    }
}

/// Shows and edits one number of the r8brain parameters. A change settles before it counts. The
/// text keeps its own form while it stands for the value, and another value from elsewhere, such
/// as from a preset, replaces typing that has not settled.
pub(crate) struct R8brainNumberField {
    audio: Entity<AudioSettings>,
    number: R8brainNumber,
    input: Entity<InputState>,
    /// The value that the text stands for
    shown: Option<f64>,
    _settle: Task<()>,
    _input_events: Subscription,
    _audio_changes: Subscription,
}

impl R8brainNumberField {
    pub(crate) fn new(
        audio: Entity<AudioSettings>,
        number: R8brainNumber,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let shown = number.value(&audio.read(cx).r8brain_parameters());
        let text = number.text(&audio.read(cx).r8brain_parameters());
        let input = cx.new(|cx| {
            let state = number.range().state(window, cx).default_value(text);
            if number == R8brainNumber::FilterFractions {
                state.placeholder("Auto")
            } else {
                state
            }
        });
        Self {
            _input_events: cx.subscribe_in(&input, window, Self::on_input_event),
            _audio_changes: cx.observe_in(&audio, window, Self::on_audio_changed),
            _settle: Task::ready(()),
            audio,
            number,
            input,
            shown,
        }
    }

    fn on_input_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                self._settle = cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(SETTLE).await;
                    this.update(cx, |this, cx| this.settle(cx)).ok();
                });
            }
            // Text that is no number gives way to the value, and `007` becomes `7`
            InputEvent::PressEnter { .. } | InputEvent::Blur => {
                self.apply(cx);
                self.show_value(window, cx);
            }
            InputEvent::Focus => {}
        }
    }

    fn on_audio_changed(
        &mut self,
        _: Entity<AudioSettings>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.number.value(&self.audio.read(cx).r8brain_parameters()) != self.shown {
            self._settle = Task::ready(());
            self.show_value(window, cx);
        }
    }

    /// Typing beyond the range waits for Enter or for leaving the field, which hold it to the range
    fn settle(&mut self, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value();
        if self.number.range().is_beyond(&text) {
            self._settle = Task::ready(());
        } else {
            self.apply(cx);
        }
    }

    fn apply(&mut self, cx: &mut Context<Self>) {
        self._settle = Task::ready(());
        let text = self.input.read(cx).value();
        if let Some(value) = self.number.parse(&text) {
            let number = self.number;
            self.audio.update(cx, |audio, cx| {
                audio.change_r8brain(|parameters| number.set(parameters, value), cx);
            });
            self.shown = number.value(&self.audio.read(cx).r8brain_parameters());
        }
    }

    fn show_value(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let parameters = self.audio.read(cx).r8brain_parameters();
        self.shown = self.number.value(&parameters);
        let text = self.number.text(&parameters);
        if self.input.read(cx).value().as_ref() != text {
            self.input
                .update(cx, |input, cx| input.set_value(text, window, cx));
        }
    }

    pub(crate) fn is_resettable(&self, cx: &App) -> bool {
        let number = self.number;
        !number.differs_between_presets()
            && number.value(&self.audio.read(cx).r8brain_parameters())
                != number.value(&R8brainParameters::default())
    }

    pub(crate) fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self._settle = Task::ready(());
        let number = self.number;
        let default = number.value(&R8brainParameters::default());
        self.audio.update(cx, |audio, cx| {
            audio.change_r8brain(|parameters| number.set(parameters, default), cx);
        });
        self.show_value(window, cx);
    }
}

impl Render for R8brainNumberField {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        NumberField::new(
            &self.input,
            self.number.range(),
            self.number.name(),
            self.number.unit(),
        )
    }
}

#[cfg(test)]
mod tests;
