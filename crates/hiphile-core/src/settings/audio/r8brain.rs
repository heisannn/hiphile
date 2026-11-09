use std::ops::RangeInclusive;
use std::sync::Arc;
use std::time::Duration;

use config::{
    R8BRAIN_ATTENUATIONS, R8BRAIN_CACHE_SIZES, R8BRAIN_FILTER_FRACTIONS, R8BRAIN_MAX_INPUT_FRAMES,
    R8BRAIN_TRANSITION_BANDS, R8brainFilter, R8brainParameters, R8brainPerformance, R8brainPhase,
    R8brainQuality,
};
use executor::Task;

use super::{AudioSettings, AudioSnapshot, State};
use crate::settings::Choice;
use crate::settings::NumberRange;

/// A field reports every keystroke, and each change restarts the resampling of what is playing.
const SETTLE: Duration = Duration::from_millis(500);

/// What the original offers for each depth. Impulse responses need less than dynamic signals.
const BIT_DEPTHS: [(&str, f64); 4] = [
    ("16-bit", 136.45),
    ("16-bit impulse response", 109.56),
    ("24-bit", 180.15),
    ("27-bit", 206.91),
];
const OTHER_BIT_DEPTH: &str = "Other";
pub const R8BRAIN_QUALITIES: [&str; 5] = ["Low", "Medium", "High", "Max", "Custom"];
pub const R8BRAIN_PHASES: [&str; 2] = ["Linear", "Minimum"];
const MINIMUM_PHASE_IX: usize = 1;

/// One of the r8brain parameters that is typed as a number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum R8brainNumber {
    Attenuation,
    TransitionBand,
    FilterFractions,
    MaxInputFrames,
    FilterCacheSize,
    FilterBankCacheSize,
}

/// One for each of [`R8brainNumber::ALL`], in its place.
pub(super) type PerNumber<T> = [T; R8brainNumber::ALL.len()];

impl R8brainNumber {
    pub const ALL: [Self; 6] = [
        Self::Attenuation,
        Self::TransitionBand,
        Self::FilterFractions,
        Self::MaxInputFrames,
        Self::FilterCacheSize,
        Self::FilterBankCacheSize,
    ];

    pub fn range(self) -> NumberRange {
        let whole = |range: RangeInclusive<u32>, step| {
            NumberRange::new(
                f64::from(*range.start()),
                f64::from(*range.end()),
                step,
                Some(0),
            )
        };
        let decimal = |range: RangeInclusive<f64>, step| {
            NumberRange::new(*range.start(), *range.end(), step, None)
        };
        match self {
            Self::Attenuation => decimal(R8BRAIN_ATTENUATIONS, 1.0),
            Self::TransitionBand => decimal(R8BRAIN_TRANSITION_BANDS, 0.5),
            Self::FilterFractions => whole(R8BRAIN_FILTER_FRACTIONS, 256.0),
            Self::MaxInputFrames => whole(R8BRAIN_MAX_INPUT_FRAMES, 256.0),
            Self::FilterCacheSize | Self::FilterBankCacheSize => whole(R8BRAIN_CACHE_SIZES, 1.0),
        }
    }

    /// What the number is, for screen readers.
    pub fn name(self) -> &'static str {
        match self {
            Self::Attenuation => "r8brain attenuation",
            Self::TransitionBand => "r8brain transition band",
            Self::FilterFractions => "r8brain filter positions",
            Self::MaxInputFrames => "r8brain input block",
            Self::FilterCacheSize => "r8brain filter cache",
            Self::FilterBankCacheSize => "r8brain filter bank cache",
        }
    }

    /// Shown after the number. Empty for a count.
    pub fn unit(self) -> &'static str {
        match self {
            Self::Attenuation => "dB",
            Self::TransitionBand => "%",
            Self::MaxInputFrames => "frames",
            Self::FilterFractions | Self::FilterCacheSize | Self::FilterBankCacheSize => "",
        }
    }

    /// What an empty field stands for, where it stands for a value.
    pub fn placeholder(self) -> Option<&'static str> {
        (self == Self::FilterFractions).then_some("Auto")
    }

    fn ix(self) -> usize {
        self as usize
    }

    /// Choosing a preset brings these back, and a reset to the default would leave the preset.
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

    /// A changed filter makes the quality custom. `value` is in the range.
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

    /// The value that `text` stands for, held to the range, or `None` if it stands for no value
    /// yet.
    fn parse(self, text: &str) -> Option<Option<f64>> {
        if text.is_empty() && self == Self::FilterFractions {
            return Some(None);
        }
        self.range().parse(text).map(Some)
    }
}

/// A part of the r8brain parameters that may go back to its default on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum R8brainSetting {
    Quality,
    Phase,
    WholeStepping,
    ExtendedFft,
    Number(R8brainNumber),
}

impl AudioSnapshot {
    pub fn r8brain_filter(&self) -> R8brainFilter {
        self.r8brain_parameters().quality.filter()
    }

    /// Into [`R8BRAIN_QUALITIES`].
    pub fn r8brain_quality_ix(&self) -> usize {
        match self.r8brain_parameters().quality {
            R8brainQuality::Low => 0,
            R8brainQuality::Medium => 1,
            R8brainQuality::High => 2,
            R8brainQuality::Max => 3,
            R8brainQuality::Custom(_) => 4,
        }
    }

    fn r8brain_bit_depth(&self) -> Option<&'static str> {
        let attenuation = self.r8brain_filter().attenuation;
        BIT_DEPTHS
            .iter()
            .find(|(_, of_depth)| *of_depth == attenuation)
            .map(|(name, _)| *name)
    }

    /// Other is on the list only while it is what the attenuation comes to.
    pub fn r8brain_bit_depth_options(&self) -> Vec<Choice> {
        BIT_DEPTHS
            .iter()
            .map(|(name, _)| *name)
            .chain(
                self.r8brain_bit_depth()
                    .is_none()
                    .then_some(OTHER_BIT_DEPTH),
            )
            .map(|name| Choice::new(name, name))
            .collect()
    }

    pub fn r8brain_bit_depth_value(&self) -> String {
        self.r8brain_bit_depth()
            .unwrap_or(OTHER_BIT_DEPTH)
            .to_owned()
    }

    /// Into [`R8BRAIN_PHASES`].
    pub fn r8brain_phase_ix(&self) -> usize {
        match self.r8brain_filter().phase {
            R8brainPhase::Linear => 0,
            R8brainPhase::Minimum => MINIMUM_PHASE_IX,
        }
    }

    /// What the number's field shows: what was typed into it, until Enter or leaving it or a
    /// change of the number from elsewhere, or else the number.
    pub fn r8brain_text(&self, number: R8brainNumber) -> String {
        self.r8brain_typed[number.ix()]
            .clone()
            .unwrap_or_else(|| number.text(&self.r8brain))
    }

    /// A number that differs between presets goes back with the preset instead.
    pub fn is_r8brain_resettable(&self, setting: R8brainSetting) -> bool {
        let parameters = self.r8brain_parameters();
        let filter = self.r8brain_filter();
        let default_filter = R8brainQuality::default().filter();
        match setting {
            R8brainSetting::Quality => parameters.quality != R8brainQuality::default(),
            R8brainSetting::Phase => filter.phase != default_filter.phase,
            R8brainSetting::WholeStepping => filter.whole_stepping != default_filter.whole_stepping,
            R8brainSetting::ExtendedFft => {
                parameters.performance.extended_fft != R8brainPerformance::default().extended_fft
            }
            R8brainSetting::Number(number) => {
                !number.differs_between_presets()
                    && number.value(&parameters) != number.value(&R8brainParameters::default())
            }
        }
    }
}

impl AudioSettings {
    /// `ix` is into [`R8BRAIN_QUALITIES`]. Custom starts from the values in use.
    pub fn select_r8brain_quality(&self, ix: usize) {
        self.change(|state| {
            let quality = match ix {
                0 => R8brainQuality::Low,
                1 => R8brainQuality::Medium,
                2 => R8brainQuality::High,
                3 => R8brainQuality::Max,
                _ => R8brainQuality::Custom(state.settings.r8brain_filter()),
            };
            state.change_r8brain(|parameters| parameters.quality = quality);
        });
    }

    /// `value` is one of [`AudioSnapshot::r8brain_bit_depth_options`].
    pub fn set_r8brain_bit_depth(&self, value: &str) {
        if let Some((_, attenuation)) = BIT_DEPTHS.iter().find(|(name, _)| *name == value) {
            self.change_r8brain_filter(|filter| filter.attenuation = *attenuation);
        }
    }

    /// `ix` is into [`R8BRAIN_PHASES`].
    pub fn select_r8brain_phase(&self, ix: usize) {
        let phase = if ix == MINIMUM_PHASE_IX {
            R8brainPhase::Minimum
        } else {
            R8brainPhase::Linear
        };
        self.change_r8brain_filter(|filter| filter.phase = phase);
    }

    pub fn set_r8brain_whole_stepping(&self, is_on: bool) {
        self.change_r8brain_filter(|filter| filter.whole_stepping = is_on);
    }

    pub fn set_r8brain_extended_fft(&self, is_on: bool) {
        self.change(|state| {
            state.change_r8brain(|parameters| parameters.performance.extended_fft = is_on);
        });
    }

    /// What the number's field holds, as it is typed. It counts once no other has come for a
    /// moment, held to the range, but a number beyond it waits for
    /// [`Self::commit_r8brain`], as `1` may be on the way to `100`.
    pub fn type_r8brain(&self, number: R8brainNumber, text: &str) {
        let this = Arc::downgrade(&self.0);
        let executor = self.0.executor.clone();
        self.0.state.update(|state| {
            state.settings.r8brain_typed[number.ix()] = Some(text.to_owned());
            state.r8brain_settles[number.ix()] = self.0.executor.spawn(async move {
                executor.timer(SETTLE).await;
                if let Some(this) = this.upgrade() {
                    AudioSettings(this).change(|state| state.settle_r8brain(number));
                }
            });
        });
    }

    /// Counts what was typed at once, held to the range, as Enter or leaving the field does. Text
    /// that is no number gives way to the number.
    pub fn commit_r8brain(&self, number: R8brainNumber) {
        self.change(|state| state.commit_r8brain(number));
    }

    /// Does nothing unless [`AudioSnapshot::is_r8brain_resettable`].
    pub fn reset_r8brain(&self, setting: R8brainSetting) {
        self.change(|state| state.reset_r8brain(setting));
    }

    /// A change makes the quality custom.
    fn change_r8brain_filter(&self, change: impl FnOnce(&mut R8brainFilter)) {
        self.change(|state| state.change_r8brain_filter(change));
    }
}

impl State {
    fn change_r8brain(&mut self, change: impl FnOnce(&mut R8brainParameters)) {
        let mut parameters = self.settings.r8brain_parameters();
        change(&mut parameters);
        self.set_r8brain_parameters(parameters);
    }

    fn change_r8brain_filter(&mut self, change: impl FnOnce(&mut R8brainFilter)) {
        let mut filter = self.settings.r8brain_filter();
        change(&mut filter);
        if filter != self.settings.r8brain_filter() {
            self.change_r8brain(|parameters| parameters.quality = R8brainQuality::Custom(filter));
        }
    }

    /// What was typed stays in the field after it counts, so that `007` reads as typed until the
    /// field is left.
    fn settle_r8brain(&mut self, number: R8brainNumber) {
        let Some(text) = self.settings.r8brain_typed[number.ix()].take() else {
            return;
        };
        if !number.range().is_beyond(&text)
            && let Some(value) = number.parse(&text)
        {
            self.change_r8brain(|parameters| number.set(parameters, value));
        }
        self.settings.r8brain_typed[number.ix()] = Some(text);
    }

    fn commit_r8brain(&mut self, number: R8brainNumber) {
        self.r8brain_settles[number.ix()] = Task::ready(());
        if let Some(text) = self.settings.r8brain_typed[number.ix()].take()
            && let Some(value) = number.parse(&text)
        {
            self.change_r8brain(|parameters| number.set(parameters, value));
        }
    }

    pub(super) fn commit_every_r8brain_number(&mut self) {
        for number in R8brainNumber::ALL {
            self.commit_r8brain(number);
        }
    }

    fn reset_r8brain(&mut self, setting: R8brainSetting) {
        if !self.settings.is_r8brain_resettable(setting) {
            return;
        }
        let default_filter = R8brainQuality::default().filter();
        match setting {
            R8brainSetting::Quality => {
                self.change_r8brain(|parameters| parameters.quality = R8brainQuality::default());
            }
            R8brainSetting::Phase => {
                self.change_r8brain_filter(|filter| filter.phase = default_filter.phase);
            }
            R8brainSetting::WholeStepping => self.change_r8brain_filter(|filter| {
                filter.whole_stepping = default_filter.whole_stepping;
            }),
            R8brainSetting::ExtendedFft => self.change_r8brain(|parameters| {
                parameters.performance.extended_fft = R8brainPerformance::default().extended_fft;
            }),
            R8brainSetting::Number(number) => {
                let default = number.value(&R8brainParameters::default());
                self.change_r8brain(|parameters| number.set(parameters, default));
            }
        }
    }

    /// A number that changed from elsewhere, as by a preset, replaces what was typed for it.
    pub(super) fn drop_r8brain_typing_changed_from(&mut self, before: &R8brainParameters) {
        let now = self.settings.r8brain_parameters();
        for number in R8brainNumber::ALL {
            if number.value(before) != number.value(&now)
                && self.settings.r8brain_typed[number.ix()].take().is_some()
            {
                self.r8brain_settles[number.ix()] = Task::ready(());
            }
        }
    }
}

#[cfg(test)]
mod tests;
