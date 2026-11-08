use config::{
    SOXR_LARGE_DFT_SIZES, SOXR_MIN_DFT_SIZES, SoxrFilter, SoxrInterpolation, SoxrParameters,
    SoxrPhase, SoxrQuality,
};
use gpui_kit::{Context, SharedString};

use crate::audio_options::{Options, number_value, numbers, same_labels};
use crate::audio_settings::AudioSettings;

pub(crate) const SOXR_QUALITIES: [&str; 5] = ["LQ", "MQ", "HQ", "VHQ", "Custom"];
pub(crate) const SOXR_PHASES: [&str; 3] = ["Linear", "Intermediate", "Minimum"];
pub(crate) const SOXR_INTERPOLATIONS: [&str; 3] = ["Auto", "Linear", "Cubic"];
/// libsoxr's precisions by bit depth
const PRECISIONS: [f64; 5] = [16.0, 20.0, 24.0, 28.0, 32.0];
const PASSBAND_ENDS: [f64; 4] = [0.90, 0.95, 0.98, 0.99];
const COEF_TABLE_KBS: [f64; 6] = [100.0, 200.0, 400.0, 800.0, 1600.0, 3200.0];

impl AudioSettings {
    pub(crate) fn change_soxr(
        &mut self,
        change: impl FnOnce(&mut SoxrParameters),
        cx: &mut Context<Self>,
    ) {
        let mut parameters = self.soxr_parameters();
        change(&mut parameters);
        self.set_soxr_parameters(parameters, cx);
    }

    pub(crate) fn change_soxr_filter(
        &mut self,
        change: impl FnOnce(&mut SoxrFilter),
        cx: &mut Context<Self>,
    ) {
        let mut filter = self.soxr_filter();
        change(&mut filter);
        if filter != self.soxr_filter() {
            self.change_soxr(
                |parameters| parameters.quality = SoxrQuality::Custom(filter),
                cx,
            );
        }
    }

    pub(crate) fn soxr_filter(&self) -> SoxrFilter {
        self.soxr_parameters().quality.filter()
    }

    pub(crate) fn soxr_quality_ix(&self) -> usize {
        match self.soxr_parameters().quality {
            SoxrQuality::Low => 0,
            SoxrQuality::Medium => 1,
            SoxrQuality::High => 2,
            SoxrQuality::VeryHigh => 3,
            SoxrQuality::Custom(_) => 4,
        }
    }

    /// `ix` is into [`SOXR_QUALITIES`]. Custom starts from the values in use, so choosing it
    /// changes nothing until one of them is changed.
    pub(crate) fn select_soxr_quality(&mut self, ix: usize, cx: &mut Context<Self>) {
        let quality = match ix {
            0 => SoxrQuality::Low,
            1 => SoxrQuality::Medium,
            2 => SoxrQuality::High,
            3 => SoxrQuality::VeryHigh,
            _ => SoxrQuality::Custom(self.soxr_filter()),
        };
        self.change_soxr(|parameters| parameters.quality = quality, cx);
    }

    pub(crate) fn soxr_precision_options(&self) -> Options {
        ascending(numbers(&PRECISIONS, self.soxr_filter().precision, |bits| {
            format!("{bits}-bit")
        }))
    }

    pub(crate) fn soxr_precision_value(&self) -> SharedString {
        number_value(self.soxr_filter().precision)
    }

    pub(crate) fn set_soxr_precision(&mut self, value: &str, cx: &mut Context<Self>) {
        if let Ok(precision) = value.parse() {
            self.change_soxr_filter(|filter| filter.precision = precision, cx);
        }
    }

    /// Three places tell the presets apart.
    pub(crate) fn soxr_passband_end_options(&self) -> Options {
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

    pub(crate) fn soxr_passband_end_value(&self) -> SharedString {
        number_value(self.soxr_filter().passband_end)
    }

    pub(crate) fn set_soxr_passband_end(&mut self, value: &str, cx: &mut Context<Self>) {
        if let Ok(passband_end) = value.parse() {
            self.change_soxr_filter(|filter| filter.passband_end = passband_end, cx);
        }
    }

    pub(crate) fn soxr_phase_ix(&self) -> usize {
        match self.soxr_filter().phase {
            SoxrPhase::Linear => 0,
            SoxrPhase::Intermediate => 1,
            SoxrPhase::Minimum => 2,
        }
    }

    /// `ix` is into [`SOXR_PHASES`].
    pub(crate) fn select_soxr_phase(&mut self, ix: usize, cx: &mut Context<Self>) {
        let phase = match ix {
            1 => SoxrPhase::Intermediate,
            2 => SoxrPhase::Minimum,
            _ => SoxrPhase::Linear,
        };
        self.change_soxr_filter(|filter| filter.phase = phase, cx);
    }

    pub(crate) fn soxr_coef_table_options(&self) -> Options {
        let current = self.soxr_parameters().performance.coef_table_kb;
        ascending(numbers(&COEF_TABLE_KBS, f64::from(current), |kb| {
            format!("{kb} kB")
        }))
    }

    pub(crate) fn soxr_coef_table_value(&self) -> SharedString {
        self.soxr_parameters()
            .performance
            .coef_table_kb
            .to_string()
            .into()
    }

    pub(crate) fn set_soxr_coef_table(&mut self, value: &str, cx: &mut Context<Self>) {
        if let Ok(kb) = value.parse() {
            self.change_soxr(|parameters| parameters.performance.coef_table_kb = kb, cx);
        }
    }

    pub(crate) fn soxr_interpolation_ix(&self) -> usize {
        match self.soxr_parameters().performance.coef_interpolation {
            SoxrInterpolation::Auto => 0,
            SoxrInterpolation::Linear => 1,
            SoxrInterpolation::Cubic => 2,
        }
    }

    /// `ix` is into [`SOXR_INTERPOLATIONS`].
    pub(crate) fn select_soxr_interpolation(&mut self, ix: usize, cx: &mut Context<Self>) {
        let interpolation = match ix {
            1 => SoxrInterpolation::Linear,
            2 => SoxrInterpolation::Cubic,
            _ => SoxrInterpolation::Auto,
        };
        self.change_soxr(
            |parameters| parameters.performance.coef_interpolation = interpolation,
            cx,
        );
    }

    pub(crate) fn soxr_min_dft_options(&self) -> Options {
        same_labels(SOXR_MIN_DFT_SIZES.iter().map(u32::to_string))
    }

    pub(crate) fn soxr_min_dft_value(&self) -> SharedString {
        self.soxr_parameters()
            .performance
            .min_dft_size
            .to_string()
            .into()
    }

    pub(crate) fn set_soxr_min_dft(&mut self, value: &str, cx: &mut Context<Self>) {
        if let Ok(size) = value.parse() {
            self.change_soxr(|parameters| parameters.performance.min_dft_size = size, cx);
        }
    }

    pub(crate) fn soxr_large_dft_options(&self) -> Options {
        same_labels(SOXR_LARGE_DFT_SIZES.iter().map(u32::to_string))
    }

    pub(crate) fn soxr_large_dft_value(&self) -> SharedString {
        self.soxr_parameters()
            .performance
            .large_dft_size
            .to_string()
            .into()
    }

    pub(crate) fn set_soxr_large_dft(&mut self, value: &str, cx: &mut Context<Self>) {
        if let Ok(size) = value.parse() {
            self.change_soxr(
                |parameters| parameters.performance.large_dft_size = size,
                cx,
            );
        }
    }
}

/// A value off the list, as every preset's pass-band end is, goes where it falls among the others.
fn ascending(mut options: Options) -> Options {
    let number = |value: &SharedString| value.parse::<f64>().unwrap_or_default();
    options.sort_by(|(a, _), (b, _)| number(a).total_cmp(&number(b)));
    options
}

#[cfg(test)]
mod tests;
