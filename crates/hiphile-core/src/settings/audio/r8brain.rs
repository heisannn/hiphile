use config::{R8brainFilter, R8brainParameters, R8brainPhase, R8brainQuality};

use super::choices::Choice;
use super::{AudioSettings, AudioSnapshot, State};

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
}

impl AudioSettings {
    pub fn change_r8brain(&self, change: impl FnOnce(&mut R8brainParameters)) {
        self.change(|state| state.change_r8brain(change));
    }

    /// A change makes the quality custom.
    pub fn change_r8brain_filter(&self, change: impl FnOnce(&mut R8brainFilter)) {
        self.change(|state| state.change_r8brain_filter(change));
    }

    /// `ix` is into [`R8BRAIN_QUALITIES`].
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
}

#[cfg(test)]
mod tests;
