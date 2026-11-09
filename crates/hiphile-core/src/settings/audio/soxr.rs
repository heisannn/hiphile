use config::{
    SOXR_LARGE_DFT_SIZES, SOXR_MIN_DFT_SIZES, SoxrFilter, SoxrInterpolation, SoxrParameters,
    SoxrPerformance, SoxrPhase, SoxrQuality,
};

use super::{AudioSettings, AudioSnapshot, State};
use crate::settings::choices::{Choice, number_value, numbers, same_labels};

pub const SOXR_QUALITIES: [&str; 5] = ["LQ", "MQ", "HQ", "VHQ", "Custom"];
pub const SOXR_PHASES: [&str; 3] = ["Linear", "Intermediate", "Minimum"];
pub const SOXR_INTERPOLATIONS: [&str; 3] = ["Auto", "Linear", "Cubic"];
/// libsoxr's precisions by bit depth
const PRECISIONS: [f64; 5] = [16.0, 20.0, 24.0, 28.0, 32.0];
const PASSBAND_ENDS: [f64; 4] = [0.90, 0.95, 0.98, 0.99];
const COEF_TABLE_KBS: [f64; 6] = [100.0, 200.0, 400.0, 800.0, 1600.0, 3200.0];

/// A part of the soxr parameters that may go back to its default on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoxrSetting {
    Quality,
    Precision,
    PassbandEnd,
    Phase,
    DoublePrecision,
    CoefTable,
    Interpolation,
    MinDft,
    LargeDft,
}

impl AudioSnapshot {
    pub fn soxr_filter(&self) -> SoxrFilter {
        self.soxr_parameters().quality.filter()
    }

    /// Into [`SOXR_QUALITIES`].
    pub fn soxr_quality_ix(&self) -> usize {
        match self.soxr_parameters().quality {
            SoxrQuality::Low => 0,
            SoxrQuality::Medium => 1,
            SoxrQuality::High => 2,
            SoxrQuality::VeryHigh => 3,
            SoxrQuality::Custom(_) => 4,
        }
    }

    pub fn soxr_precision_options(&self) -> Vec<Choice> {
        ascending(numbers(&PRECISIONS, self.soxr_filter().precision, |bits| {
            format!("{bits}-bit")
        }))
    }

    pub fn soxr_precision_value(&self) -> String {
        number_value(self.soxr_filter().precision)
    }

    /// Three places tell the presets apart.
    pub fn soxr_passband_end_options(&self) -> Vec<Choice> {
        ascending(numbers(
            &PASSBAND_ENDS,
            self.soxr_filter().passband_end,
            |end| {
                let mut label = format!("{end:.3}");
                if label.ends_with('0') {
                    label.pop();
                }
                label
            },
        ))
    }

    pub fn soxr_passband_end_value(&self) -> String {
        number_value(self.soxr_filter().passband_end)
    }

    /// Into [`SOXR_PHASES`].
    pub fn soxr_phase_ix(&self) -> usize {
        match self.soxr_filter().phase {
            SoxrPhase::Linear => 0,
            SoxrPhase::Intermediate => 1,
            SoxrPhase::Minimum => 2,
        }
    }

    pub fn soxr_coef_table_options(&self) -> Vec<Choice> {
        let current = self.soxr_parameters().performance.coef_table_kb;
        ascending(numbers(&COEF_TABLE_KBS, f64::from(current), |kb| {
            format!("{kb} kB")
        }))
    }

    pub fn soxr_coef_table_value(&self) -> String {
        self.soxr_parameters().performance.coef_table_kb.to_string()
    }

    /// Into [`SOXR_INTERPOLATIONS`].
    pub fn soxr_interpolation_ix(&self) -> usize {
        match self.soxr_parameters().performance.coef_interpolation {
            SoxrInterpolation::Auto => 0,
            SoxrInterpolation::Linear => 1,
            SoxrInterpolation::Cubic => 2,
        }
    }

    pub fn soxr_min_dft_options(&self) -> Vec<Choice> {
        same_labels(SOXR_MIN_DFT_SIZES.iter().map(u32::to_string))
    }

    pub fn soxr_min_dft_value(&self) -> String {
        self.soxr_parameters().performance.min_dft_size.to_string()
    }

    pub fn soxr_large_dft_options(&self) -> Vec<Choice> {
        same_labels(SOXR_LARGE_DFT_SIZES.iter().map(u32::to_string))
    }

    pub fn soxr_large_dft_value(&self) -> String {
        self.soxr_parameters()
            .performance
            .large_dft_size
            .to_string()
    }

    /// The precision and the pass-band end differ between presets, so they go back with the
    /// preset instead.
    pub fn is_soxr_resettable(&self, setting: SoxrSetting) -> bool {
        let parameters = self.soxr_parameters();
        let (filter, performance) = (self.soxr_filter(), parameters.performance);
        let (default_filter, default_performance) = defaults();
        match setting {
            SoxrSetting::Quality => parameters.quality != SoxrQuality::default(),
            SoxrSetting::Precision | SoxrSetting::PassbandEnd => false,
            SoxrSetting::Phase => filter.phase != default_filter.phase,
            SoxrSetting::DoublePrecision => {
                filter.double_precision != default_filter.double_precision
            }
            SoxrSetting::CoefTable => {
                performance.coef_table_kb != default_performance.coef_table_kb
            }
            SoxrSetting::Interpolation => {
                performance.coef_interpolation != default_performance.coef_interpolation
            }
            SoxrSetting::MinDft => performance.min_dft_size != default_performance.min_dft_size,
            SoxrSetting::LargeDft => {
                performance.large_dft_size != default_performance.large_dft_size
            }
        }
    }
}

impl AudioSettings {
    fn change_soxr(&self, change: impl FnOnce(&mut SoxrParameters)) {
        self.change(|state| state.change_soxr(change));
    }

    /// A change makes the quality custom.
    fn change_soxr_filter(&self, change: impl FnOnce(&mut SoxrFilter)) {
        self.change(|state| state.change_soxr_filter(change));
    }

    /// `ix` is into [`SOXR_QUALITIES`]. Custom starts from the values in use, so choosing it
    /// changes nothing until one of them is changed.
    pub fn select_soxr_quality(&self, ix: usize) {
        self.change(|state| {
            let quality = match ix {
                0 => SoxrQuality::Low,
                1 => SoxrQuality::Medium,
                2 => SoxrQuality::High,
                3 => SoxrQuality::VeryHigh,
                _ => SoxrQuality::Custom(state.settings.soxr_filter()),
            };
            state.change_soxr(|parameters| parameters.quality = quality);
        });
    }

    pub fn set_soxr_precision(&self, value: &str) {
        if let Ok(precision) = value.parse() {
            self.change_soxr_filter(|filter| filter.precision = precision);
        }
    }

    pub fn set_soxr_passband_end(&self, value: &str) {
        if let Ok(passband_end) = value.parse() {
            self.change_soxr_filter(|filter| filter.passband_end = passband_end);
        }
    }

    /// `ix` is into [`SOXR_PHASES`].
    pub fn select_soxr_phase(&self, ix: usize) {
        let phase = match ix {
            1 => SoxrPhase::Intermediate,
            2 => SoxrPhase::Minimum,
            _ => SoxrPhase::Linear,
        };
        self.change_soxr_filter(|filter| filter.phase = phase);
    }

    pub fn set_soxr_coef_table(&self, value: &str) {
        if let Ok(kb) = value.parse() {
            self.change_soxr(|parameters| parameters.performance.coef_table_kb = kb);
        }
    }

    /// `ix` is into [`SOXR_INTERPOLATIONS`].
    pub fn select_soxr_interpolation(&self, ix: usize) {
        let interpolation = match ix {
            1 => SoxrInterpolation::Linear,
            2 => SoxrInterpolation::Cubic,
            _ => SoxrInterpolation::Auto,
        };
        self.change_soxr(|parameters| parameters.performance.coef_interpolation = interpolation);
    }

    pub fn set_soxr_min_dft(&self, value: &str) {
        if let Ok(size) = value.parse() {
            self.change_soxr(|parameters| parameters.performance.min_dft_size = size);
        }
    }

    pub fn set_soxr_large_dft(&self, value: &str) {
        if let Ok(size) = value.parse() {
            self.change_soxr(|parameters| parameters.performance.large_dft_size = size);
        }
    }

    /// A change makes the quality custom.
    pub fn set_soxr_double_precision(&self, is_on: bool) {
        self.change_soxr_filter(|filter| filter.double_precision = is_on);
    }

    /// Does nothing unless [`AudioSnapshot::is_soxr_resettable`].
    pub fn reset_soxr(&self, setting: SoxrSetting) {
        self.change(|state| state.reset_soxr(setting));
    }
}

impl State {
    fn change_soxr(&mut self, change: impl FnOnce(&mut SoxrParameters)) {
        let mut parameters = self.settings.soxr_parameters();
        change(&mut parameters);
        self.set_soxr_parameters(parameters);
    }

    fn change_soxr_filter(&mut self, change: impl FnOnce(&mut SoxrFilter)) {
        let mut filter = self.settings.soxr_filter();
        change(&mut filter);
        if filter != self.settings.soxr_filter() {
            self.change_soxr(|parameters| parameters.quality = SoxrQuality::Custom(filter));
        }
    }

    fn reset_soxr(&mut self, setting: SoxrSetting) {
        if !self.settings.is_soxr_resettable(setting) {
            return;
        }
        let (filter, performance) = defaults();
        match setting {
            SoxrSetting::Quality => {
                self.change_soxr(|parameters| parameters.quality = SoxrQuality::default());
            }
            SoxrSetting::Precision | SoxrSetting::PassbandEnd => {}
            SoxrSetting::Phase => self.change_soxr_filter(|current| current.phase = filter.phase),
            SoxrSetting::DoublePrecision => self.change_soxr_filter(|current| {
                current.double_precision = filter.double_precision;
            }),
            SoxrSetting::CoefTable => self.change_soxr(|parameters| {
                parameters.performance.coef_table_kb = performance.coef_table_kb;
            }),
            SoxrSetting::Interpolation => self.change_soxr(|parameters| {
                parameters.performance.coef_interpolation = performance.coef_interpolation;
            }),
            SoxrSetting::MinDft => self.change_soxr(|parameters| {
                parameters.performance.min_dft_size = performance.min_dft_size;
            }),
            SoxrSetting::LargeDft => self.change_soxr(|parameters| {
                parameters.performance.large_dft_size = performance.large_dft_size;
            }),
        }
    }
}

fn defaults() -> (SoxrFilter, SoxrPerformance) {
    (SoxrQuality::default().filter(), SoxrPerformance::default())
}

/// A value off the list, as every preset's pass-band end is, goes where it falls among the others.
fn ascending(mut options: Vec<Choice>) -> Vec<Choice> {
    let number = |choice: &Choice| choice.value().parse::<f64>().unwrap_or_default();
    options.sort_by(|a, b| number(a).total_cmp(&number(b)));
    options
}

#[cfg(test)]
mod tests;
