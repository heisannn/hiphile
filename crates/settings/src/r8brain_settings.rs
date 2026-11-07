use config::{R8brainFilter, R8brainParameters, R8brainPhase, R8brainQuality};
use gpui_kit::{Context, SharedString};

use crate::audio_settings::AudioSettings;

/// What the original offers for each depth. Impulse responses need less than
/// dynamic signals.
const BIT_DEPTHS: [(&str, f64); 4] = [
    ("16-bit", 136.45),
    ("16-bit impulse response", 109.56),
    ("24-bit", 180.15),
    ("27-bit", 206.91),
];
const OTHER_BIT_DEPTH: &str = "Other";
pub(crate) const R8BRAIN_QUALITIES: [&str; 5] = ["Low", "Medium", "High", "Max", "Custom"];
pub(crate) const PHASES: [&str; 2] = ["Linear", "Minimum"];
const MINIMUM_PHASE_IX: usize = 1;

impl AudioSettings {
    pub(crate) fn change_r8brain(
        &mut self,
        change: impl FnOnce(&mut R8brainParameters),
        cx: &mut Context<Self>,
    ) {
        let mut parameters = self.r8brain_parameters();
        change(&mut parameters);
        self.set_r8brain_parameters(parameters, cx);
    }

    pub(crate) fn change_r8brain_filter(
        &mut self,
        change: impl FnOnce(&mut R8brainFilter),
        cx: &mut Context<Self>,
    ) {
        let mut filter = self.r8brain_filter();
        change(&mut filter);
        if filter != self.r8brain_filter() {
            self.change_r8brain(
                |parameters| parameters.quality = R8brainQuality::Custom(filter),
                cx,
            );
        }
    }

    pub(crate) fn r8brain_filter(&self) -> R8brainFilter {
        self.r8brain_parameters().quality.filter()
    }

    pub(crate) fn r8brain_quality_ix(&self) -> usize {
        match self.r8brain_parameters().quality {
            R8brainQuality::Low => 0,
            R8brainQuality::Medium => 1,
            R8brainQuality::High => 2,
            R8brainQuality::Max => 3,
            R8brainQuality::Custom(_) => 4,
        }
    }

    /// `ix` is into [`R8BRAIN_QUALITIES`].
    pub(crate) fn select_r8brain_quality(&mut self, ix: usize, cx: &mut Context<Self>) {
        let quality = match ix {
            0 => R8brainQuality::Low,
            1 => R8brainQuality::Medium,
            2 => R8brainQuality::High,
            3 => R8brainQuality::Max,
            _ => R8brainQuality::Custom(self.r8brain_filter()),
        };
        self.change_r8brain(|parameters| parameters.quality = quality, cx);
    }

    fn r8brain_bit_depth(&self) -> Option<&'static str> {
        let attenuation = self.r8brain_filter().attenuation;
        BIT_DEPTHS
            .iter()
            .find(|(_, of_depth)| *of_depth == attenuation)
            .map(|(name, _)| *name)
    }

    /// Other is on the list only while it is what the attenuation comes to.
    pub(crate) fn r8brain_bit_depth_options(&self) -> Vec<(SharedString, SharedString)> {
        BIT_DEPTHS
            .iter()
            .map(|(name, _)| *name)
            .chain(
                self.r8brain_bit_depth()
                    .is_none()
                    .then_some(OTHER_BIT_DEPTH),
            )
            .map(|name| (name.into(), name.into()))
            .collect()
    }

    pub(crate) fn r8brain_bit_depth_value(&self) -> SharedString {
        self.r8brain_bit_depth().unwrap_or(OTHER_BIT_DEPTH).into()
    }

    pub(crate) fn set_r8brain_bit_depth(&mut self, value: &str, cx: &mut Context<Self>) {
        if let Some((_, attenuation)) = BIT_DEPTHS.iter().find(|(name, _)| *name == value) {
            self.change_r8brain_filter(|filter| filter.attenuation = *attenuation, cx);
        }
    }

    pub(crate) fn r8brain_phase_ix(&self) -> usize {
        match self.r8brain_filter().phase {
            R8brainPhase::Linear => 0,
            R8brainPhase::Minimum => MINIMUM_PHASE_IX,
        }
    }

    pub(crate) fn select_r8brain_phase(&mut self, ix: usize, cx: &mut Context<Self>) {
        let phase = if ix == MINIMUM_PHASE_IX {
            R8brainPhase::Minimum
        } else {
            R8brainPhase::Linear
        };
        self.change_r8brain_filter(|filter| filter.phase = phase, cx);
    }
}

#[cfg(test)]
mod tests;
