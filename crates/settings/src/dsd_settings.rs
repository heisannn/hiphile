use audio::{CAN_PLAY_DOP, DopRate};
use config::{
    DSD_ORDERS, DSD_SINC_LENGTHS, DsdFilter, DsdParameters, DsdQuality, DsdRate,
    RESAMPLING_FFT_SIZES, ResamplingWindow,
};
use gpui_kit::{Context, SharedString};

use crate::audio_settings::{AudioSettings, window_label};

const DSD_QUALITIES: [&str; 5] = ["Low", "Medium", "High", "Max", "Custom"];

const SINC: &str = "sinc";
const FFT: &str = "fft";
const AUTO: &str = "auto";
const OFF: &str = "off";
const DEFAULT_FFT_SIZE: u32 = 4096;
const DEFAULT_SINC_LENGTH: u32 = 1024;
const CUTOFFS: [f64; 5] = [0.80, 0.85, 0.90, 0.95, 0.98];
const NOISE_GAINS: [f64; 4] = [1.30, 1.35, 1.40, 1.45];
const NARROW_NOISE_BAND: &str = "20000";
const WIDE_NOISE_BAND: &str = "48000";
const DITHER_DBS: [f64; 5] = [-140.0, -120.0, -100.0, -80.0, -60.0];
const INPUT_LEVEL_DBS: [f64; 6] = [-12.0, -9.0, -6.0, -3.0, 0.0, 3.0];
const LIMITER_CEILING_DBS: [f64; 3] = [1.0, 0.0, -1.0];

type Options = Vec<(SharedString, SharedString)>;

impl AudioSettings {
    pub(crate) fn is_dsd_offered() -> bool {
        CAN_PLAY_DOP
    }

    pub(crate) fn is_dsd_on(&self) -> bool {
        self.output().dop().is_some()
    }

    /// Why DSD cannot be turned on for the chosen device.
    pub(crate) fn dsd_obstacle(&self) -> Option<&'static str> {
        if self.is_dsd_on() {
            return None;
        }
        if self.output().device().is_none() {
            return Some(
                "Choose an output device above. DSD doesn't go to the system default, which can \
                 change to a device that plays it as loud noise.",
            );
        }
        match self.selected_device() {
            None => Some("The output device isn't connected."),
            Some(device) if device.dop_rates().is_empty() => {
                Some("This device doesn't take 24-bit audio or better at any DSD rate.")
            }
            Some(_) => None,
        }
    }

    /// Asks for no confirmation itself: see the warning in the settings view.
    pub(crate) fn turn_dsd_on(&mut self, cx: &mut Context<Self>) {
        let mut dsd = self.dsd().clone();
        dsd.is_on = true;
        dsd.device = self.output().device().map(str::to_owned);
        dsd.rate = Some(
            dsd.rate
                .filter(|rate| self.has_dsd_rate(*rate))
                .unwrap_or_else(|| self.highest_dsd_rate()),
        );
        self.set_dsd(dsd, cx);
    }

    pub(crate) fn turn_dsd_off(&mut self, cx: &mut Context<Self>) {
        let mut dsd = self.dsd().clone();
        dsd.is_on = false;
        self.set_dsd(dsd, cx);
    }

    pub(crate) fn dsd_device_name(&self) -> SharedString {
        self.selected_device()
            .map_or("this device", |device| device.name())
            .to_owned()
            .into()
    }

    fn dop_rates(&self) -> Option<&[u32]> {
        self.selected_device().map(audio::OutputDevice::dop_rates)
    }

    /// Without a listed device, nothing is known to be unsupported.
    fn has_dsd_rate(&self, rate: DsdRate) -> bool {
        self.dop_rates()
            .is_none_or(|rates| is_rate_supported(rates, rate))
    }

    fn highest_dsd_rate(&self) -> DsdRate {
        DsdRate::ALL
            .into_iter()
            .rev()
            .find(|rate| self.has_dsd_rate(*rate))
            .unwrap_or(DsdRate::Dsd64)
    }

    pub(crate) fn dsd_rate_options(&self) -> Options {
        DsdRate::ALL
            .into_iter()
            .map(|rate| {
                let name = format!("DSD{}", rate.multiple());
                let label = if self.has_dsd_rate(rate) {
                    name
                } else {
                    format!("{name} (not supported)")
                };
                (rate.multiple().to_string().into(), label.into())
            })
            .collect()
    }

    pub(crate) fn dsd_rate_value(&self) -> SharedString {
        self.dsd()
            .rate
            .unwrap_or(DsdRate::Dsd256)
            .multiple()
            .to_string()
            .into()
    }

    pub(crate) fn set_dsd_rate(&mut self, value: &str, cx: &mut Context<Self>) {
        let rate = DsdRate::ALL
            .into_iter()
            .find(|rate| rate.multiple().to_string() == value);
        if rate.is_some() {
            let mut dsd = self.dsd().clone();
            dsd.rate = rate;
            self.set_dsd(dsd, cx);
        }
    }

    pub(crate) fn matches_song_family(&self) -> bool {
        self.dsd().matches_song_family
    }

    /// Asks for no confirmation itself: see the warning in the settings view.
    pub(crate) fn set_matches_song_family(&mut self, matches: bool, cx: &mut Context<Self>) {
        let mut dsd = self.dsd().clone();
        dsd.matches_song_family = matches;
        self.set_dsd(dsd, cx);
    }

    pub(crate) fn fades_dsd_in(&self) -> bool {
        self.dsd().fades_in
    }

    pub(crate) fn set_fades_dsd_in(&mut self, is_on: bool, cx: &mut Context<Self>) {
        let mut dsd = self.dsd().clone();
        dsd.fades_in = is_on;
        self.set_dsd(dsd, cx);
    }

    pub(crate) fn dsd_quality_options(&self) -> Options {
        same_labels(DSD_QUALITIES.iter().map(|name| (*name).to_owned()))
    }

    pub(crate) fn dsd_quality_value(&self) -> SharedString {
        DSD_QUALITIES[self.dsd_quality_ix()].into()
    }

    pub(crate) fn set_dsd_quality(&mut self, value: &str, cx: &mut Context<Self>) {
        if let Some(ix) = DSD_QUALITIES.iter().position(|name| *name == value) {
            self.select_dsd_quality(ix, cx);
        }
    }

    pub(crate) fn dsd_quality_ix(&self) -> usize {
        match self.dsd().quality {
            DsdQuality::Low => 0,
            DsdQuality::Medium => 1,
            DsdQuality::High => 2,
            DsdQuality::Max => 3,
            DsdQuality::Custom(_) => 4,
        }
    }

    /// `ix` is into [`DSD_QUALITIES`]. Custom starts from the values in use, so
    /// choosing it changes nothing until one of them is changed.
    pub(crate) fn select_dsd_quality(&mut self, ix: usize, cx: &mut Context<Self>) {
        let mut dsd = self.dsd().clone();
        dsd.quality = match ix {
            0 => DsdQuality::Low,
            1 => DsdQuality::Medium,
            2 => DsdQuality::High,
            3 => DsdQuality::Max,
            _ => DsdQuality::Custom(dsd.quality.parameters()),
        };
        self.set_dsd(dsd, cx);
    }

    fn dsd_parameters(&self) -> DsdParameters {
        self.dsd().quality.parameters()
    }

    /// A change makes the quality custom.
    fn change_dsd_parameters(
        &mut self,
        change: impl FnOnce(&mut DsdParameters),
        cx: &mut Context<Self>,
    ) {
        let mut parameters = self.dsd_parameters();
        change(&mut parameters);
        if parameters != self.dsd_parameters() {
            let mut dsd = self.dsd().clone();
            dsd.quality = DsdQuality::Custom(parameters);
            self.set_dsd(dsd, cx);
        }
    }

    pub(crate) fn dsd_uses_sinc(&self) -> bool {
        matches!(self.dsd_parameters().filter, DsdFilter::Sinc { .. })
    }

    pub(crate) fn dsd_filter_options(&self) -> Options {
        vec![(SINC.into(), "Sinc".into()), (FFT.into(), "FFT".into())]
    }

    pub(crate) fn dsd_filter_value(&self) -> SharedString {
        if self.dsd_uses_sinc() { SINC } else { FFT }.into()
    }

    pub(crate) fn set_dsd_filter(&mut self, value: &str, cx: &mut Context<Self>) {
        if self.dsd_filter_value() == value {
            return;
        }
        let window = self.dsd_parameters().filter.window();
        let filter = if value == SINC {
            DsdFilter::Sinc {
                length: DEFAULT_SINC_LENGTH,
                window,
                cutoff: None,
            }
        } else {
            DsdFilter::Fft {
                size: DEFAULT_FFT_SIZE,
                window,
            }
        };
        self.change_dsd_parameters(|parameters| parameters.filter = filter, cx);
    }

    pub(crate) fn dsd_sinc_length_options(&self) -> Options {
        same_labels(DSD_SINC_LENGTHS.iter().map(u32::to_string))
    }

    pub(crate) fn dsd_sinc_length_value(&self) -> SharedString {
        match self.dsd_parameters().filter {
            DsdFilter::Sinc { length, .. } => length,
            DsdFilter::Fft { .. } => DEFAULT_SINC_LENGTH,
        }
        .to_string()
        .into()
    }

    pub(crate) fn set_dsd_sinc_length(&mut self, value: &str, cx: &mut Context<Self>) {
        let Ok(length) = value.parse() else {
            return;
        };
        self.change_dsd_parameters(
            |parameters| {
                if let DsdFilter::Sinc {
                    length: current, ..
                } = &mut parameters.filter
                {
                    *current = length;
                }
            },
            cx,
        );
    }

    pub(crate) fn dsd_fft_size_options(&self) -> Options {
        same_labels(RESAMPLING_FFT_SIZES.iter().map(u32::to_string))
    }

    pub(crate) fn dsd_fft_size_value(&self) -> SharedString {
        match self.dsd_parameters().filter {
            DsdFilter::Fft { size, .. } => size,
            DsdFilter::Sinc { .. } => DEFAULT_FFT_SIZE,
        }
        .to_string()
        .into()
    }

    pub(crate) fn set_dsd_fft_size(&mut self, value: &str, cx: &mut Context<Self>) {
        let Ok(size) = value.parse() else {
            return;
        };
        self.change_dsd_parameters(
            |parameters| {
                if let DsdFilter::Fft { size: current, .. } = &mut parameters.filter {
                    *current = size;
                }
            },
            cx,
        );
    }

    pub(crate) fn dsd_window_value(&self) -> SharedString {
        window_label(self.dsd_parameters().filter.window()).into()
    }

    pub(crate) fn set_dsd_window(&mut self, value: &str, cx: &mut Context<Self>) {
        let window = ResamplingWindow::ALL
            .into_iter()
            .find(|window| window_label(*window) == value);
        if let Some(window) = window {
            self.change_dsd_parameters(
                |parameters| parameters.filter = parameters.filter.with_window(window),
                cx,
            );
        }
    }

    pub(crate) fn dsd_cutoff_options(&self) -> Options {
        numbers_or(AUTO, "Auto", &CUTOFFS, self.dsd_cutoff(), |cutoff| {
            format!("{cutoff:.2}")
        })
    }

    fn dsd_cutoff(&self) -> Option<f64> {
        match self.dsd_parameters().filter {
            DsdFilter::Sinc { cutoff, .. } => cutoff,
            DsdFilter::Fft { .. } => None,
        }
    }

    pub(crate) fn dsd_cutoff_value(&self) -> SharedString {
        self.dsd_cutoff().map_or(AUTO.into(), number_value)
    }

    pub(crate) fn set_dsd_cutoff(&mut self, value: &str, cx: &mut Context<Self>) {
        let cutoff = value.parse().ok();
        self.change_dsd_parameters(
            |parameters| {
                if let DsdFilter::Sinc {
                    cutoff: current, ..
                } = &mut parameters.filter
                {
                    *current = cutoff;
                }
            },
            cx,
        );
    }

    pub(crate) fn dsd_order_options(&self) -> Options {
        same_labels(DSD_ORDERS.iter().map(u8::to_string))
    }

    pub(crate) fn dsd_order_value(&self) -> SharedString {
        self.dsd_parameters().order.to_string().into()
    }

    pub(crate) fn set_dsd_order(&mut self, value: &str, cx: &mut Context<Self>) {
        if let Ok(order) = value.parse() {
            self.change_dsd_parameters(|parameters| parameters.order = order, cx);
        }
    }

    pub(crate) fn dsd_noise_gain_options(&self) -> Options {
        numbers(&NOISE_GAINS, self.dsd_parameters().noise_gain, |gain| {
            format!("{gain:.2}")
        })
    }

    pub(crate) fn dsd_noise_gain_value(&self) -> SharedString {
        number_value(self.dsd_parameters().noise_gain)
    }

    pub(crate) fn set_dsd_noise_gain(&mut self, value: &str, cx: &mut Context<Self>) {
        if let Ok(noise_gain) = value.parse() {
            self.change_dsd_parameters(|parameters| parameters.noise_gain = noise_gain, cx);
        }
    }

    pub(crate) fn dsd_noise_band_options(&self) -> Options {
        vec![
            (NARROW_NOISE_BAND.into(), "20 kHz".into()),
            (WIDE_NOISE_BAND.into(), "48 kHz".into()),
        ]
    }

    pub(crate) fn dsd_noise_band_value(&self) -> SharedString {
        if self.dsd_parameters().wide_noise_band && self.can_widen_dsd_noise_band() {
            WIDE_NOISE_BAND
        } else {
            NARROW_NOISE_BAND
        }
        .into()
    }

    pub(crate) fn set_dsd_noise_band(&mut self, value: &str, cx: &mut Context<Self>) {
        self.change_dsd_parameters(
            |parameters| parameters.wide_noise_band = value == WIDE_NOISE_BAND,
            cx,
        );
    }

    pub(crate) fn can_widen_dsd_noise_band(&self) -> bool {
        self.dsd().rate.unwrap_or(DsdRate::Dsd256) == DsdRate::Dsd256
    }

    pub(crate) fn has_optimized_dsd_zeros(&self) -> bool {
        self.dsd_parameters().optimized_zeros
    }

    pub(crate) fn set_optimized_dsd_zeros(&mut self, is_on: bool, cx: &mut Context<Self>) {
        self.change_dsd_parameters(|parameters| parameters.optimized_zeros = is_on, cx);
    }

    pub(crate) fn dsd_dither_options(&self) -> Options {
        numbers_or(
            OFF,
            "Off",
            &DITHER_DBS,
            self.dsd_parameters().dither_db,
            |db| format!("{db:.0} dB"),
        )
    }

    pub(crate) fn dsd_dither_value(&self) -> SharedString {
        self.dsd_parameters()
            .dither_db
            .map_or(OFF.into(), number_value)
    }

    pub(crate) fn set_dsd_dither(&mut self, value: &str, cx: &mut Context<Self>) {
        let dither_db = value.parse().ok();
        self.change_dsd_parameters(|parameters| parameters.dither_db = dither_db, cx);
    }

    pub(crate) fn dsd_input_level_options(&self) -> Options {
        let current = self.dsd_parameters().input_level_db;
        numbers(&INPUT_LEVEL_DBS, current, |db| format!("{db:+.0} dB"))
    }

    pub(crate) fn dsd_input_level_value(&self) -> SharedString {
        number_value(self.dsd_parameters().input_level_db)
    }

    pub(crate) fn set_dsd_input_level(&mut self, value: &str, cx: &mut Context<Self>) {
        if let Ok(input_level_db) = value.parse() {
            self.change_dsd_parameters(|parameters| parameters.input_level_db = input_level_db, cx);
        }
    }

    pub(crate) fn dsd_limiter_options(&self) -> Options {
        let current = self.dsd_parameters().limiter_ceiling_db;
        numbers_or(OFF, "Off", &LIMITER_CEILING_DBS, current, |db| {
            format!("{db:+.0} dBTP")
        })
    }

    pub(crate) fn dsd_limiter_value(&self) -> SharedString {
        self.dsd_parameters()
            .limiter_ceiling_db
            .map_or(OFF.into(), number_value)
    }

    pub(crate) fn set_dsd_limiter(&mut self, value: &str, cx: &mut Context<Self>) {
        let ceiling = value.parse().ok();
        self.change_dsd_parameters(|parameters| parameters.limiter_ceiling_db = ceiling, cx);
    }
}

/// A DSD rate counts when either family's frame rate fits, as the other family
/// stands in for the one a device lacks.
fn is_rate_supported(frame_rates: &[u32], rate: DsdRate) -> bool {
    let rate = match rate {
        DsdRate::Dsd64 => DopRate::Dsd64,
        DsdRate::Dsd128 => DopRate::Dsd128,
        DsdRate::Dsd256 => DopRate::Dsd256,
    };
    frame_rates.contains(&rate.frame_rate_for(44_100))
        || frame_rates.contains(&rate.frame_rate_for(48_000))
}

fn same_labels(values: impl Iterator<Item = String>) -> Options {
    values
        .map(|value| {
            let value = SharedString::from(value);
            (value.clone(), value)
        })
        .collect()
}

fn number_value(number: f64) -> SharedString {
    number.to_string().into()
}

/// The values, and `current` too if the file holds one that is not among them,
/// so that the choice stays visible.
fn numbers(values: &[f64], current: f64, label: impl Fn(f64) -> String) -> Options {
    let mut options: Options = values
        .iter()
        .map(|number| (number_value(*number), label(*number).into()))
        .collect();
    if !values.contains(&current) {
        options.push((number_value(current), label(current).into()));
    }
    options
}

fn numbers_or(
    none_value: &'static str,
    none_label: &'static str,
    values: &[f64],
    current: Option<f64>,
    label: impl Fn(f64) -> String,
) -> Options {
    let mut options: Options = vec![(none_value.into(), none_label.into())];
    match current {
        Some(current) => options.extend(numbers(values, current, label)),
        None => options.extend(
            values
                .iter()
                .map(|number| (number_value(*number), label(*number).into())),
        ),
    }
    options
}

#[cfg(test)]
mod tests {
    use gpui_kit::TestAppContext;

    use super::*;
    use crate::audio_settings::tests::settings_saving_to;

    const DAC: &str = "coreaudio:dac";

    #[gpui_kit::test]
    fn dsd_goes_on_for_the_chosen_device_and_is_saved(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let (settings, told, _subscription) = settings_saving_to(&store, cx);

        settings.update(cx, |settings, cx| {
            settings.set_device(DAC, cx);
            settings.turn_dsd_on(cx);
        });
        cx.run_until_parked();

        assert_eq!(told.get(), 2);
        let saved = store.dsd_output().unwrap();
        assert!(saved.is_on);
        assert_eq!(saved.device.as_deref(), Some(DAC));
        assert_eq!(saved.rate, Some(DsdRate::Dsd256));
        settings.read_with(cx, |settings, _| {
            assert!(settings.is_dsd_on());
            assert!(settings.output().dop().is_some());
        });
    }

    #[gpui_kit::test]
    fn another_device_plays_pcm_until_dsd_is_turned_on_for_it(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let (settings, _told, _subscription) = settings_saving_to(&store, cx);
        settings.update(cx, |settings, cx| {
            settings.set_device(DAC, cx);
            settings.turn_dsd_on(cx);
            settings.set_device("coreaudio:other", cx);
        });
        settings.read_with(cx, |settings, _| assert!(!settings.is_dsd_on()));

        settings.update(cx, |settings, cx| settings.set_device(DAC, cx));
        settings.read_with(cx, |settings, _| assert!(settings.is_dsd_on()));
    }

    #[gpui_kit::test]
    fn the_fade_can_be_turned_off_and_reaches_the_output(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let (settings, told, _subscription) = settings_saving_to(&store, cx);
        settings.update(cx, |settings, cx| {
            settings.set_device(DAC, cx);
            settings.turn_dsd_on(cx);
        });
        settings.read_with(cx, |settings, _| {
            assert!(settings.fades_dsd_in());
            assert!(settings.output().dop_fades_in());
        });
        let before = told.get();

        settings.update(cx, |settings, cx| settings.set_fades_dsd_in(false, cx));
        cx.run_until_parked();

        assert_eq!(told.get(), before + 1);
        assert!(!store.dsd_output().unwrap().fades_in);
        settings.read_with(cx, |settings, _| {
            assert!(!settings.fades_dsd_in());
            assert!(!settings.output().dop_fades_in());
        });
    }

    #[gpui_kit::test]
    fn max_reaches_the_output_and_stays_selected(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let (settings, _told, _subscription) = settings_saving_to(&store, cx);
        settings.update(cx, |settings, cx| settings.set_dsd_quality("Max", cx));
        cx.run_until_parked();
        assert_eq!(store.dsd_output().unwrap().quality, DsdQuality::Max);
        settings.read_with(cx, |settings, _| {
            assert_eq!(settings.dsd_quality_value().as_ref(), "Max");
            assert_eq!(settings.dsd_sinc_length_value().as_ref(), "2048");
        });
    }

    #[gpui_kit::test]
    fn a_chosen_cutoff_is_listed_once(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let (settings, _told, _subscription) = settings_saving_to(&store, cx);
        settings.update(cx, |settings, cx| {
            settings.set_device(DAC, cx);
            settings.turn_dsd_on(cx);
            settings.set_dsd_cutoff("0.85", cx);
        });
        settings.read_with(cx, |settings, _| {
            assert_eq!(settings.dsd_cutoff_value().as_ref(), "0.85");
            assert_eq!(settings.dsd_cutoff_options().len(), CUTOFFS.len() + 1);
        });
    }

    #[gpui_kit::test]
    fn the_wide_noise_band_shows_only_where_it_applies(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let (settings, _told, _subscription) = settings_saving_to(&store, cx);
        settings.update(cx, |settings, cx| {
            settings.set_device(DAC, cx);
            settings.turn_dsd_on(cx);
            settings.set_dsd_noise_band(WIDE_NOISE_BAND, cx);
        });
        settings.read_with(cx, |settings, _| {
            assert_eq!(settings.dsd_noise_band_value().as_ref(), WIDE_NOISE_BAND);
        });
        settings.update(cx, |settings, cx| settings.set_dsd_rate("64", cx));
        settings.read_with(cx, |settings, _| {
            assert_eq!(settings.dsd_noise_band_value().as_ref(), NARROW_NOISE_BAND);
        });
    }

    #[gpui_kit::test]
    fn the_system_default_can_have_no_dsd(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let (settings, _told, _subscription) = settings_saving_to(&store, cx);
        settings.read_with(cx, |settings, _| {
            assert!(settings.dsd_obstacle().is_some());
        });
        settings.update(cx, |settings, cx| settings.turn_dsd_on(cx));
        settings.read_with(cx, |settings, _| assert!(!settings.is_dsd_on()));
    }

    #[gpui_kit::test]
    fn a_new_rate_or_preset_reaches_the_output(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let (settings, told, _subscription) = settings_saving_to(&store, cx);
        settings.update(cx, |settings, cx| {
            settings.set_device(DAC, cx);
            settings.turn_dsd_on(cx);
        });
        let before = told.get();

        settings.update(cx, |settings, cx| settings.set_dsd_rate("64", cx));
        settings.read_with(cx, |settings, _| {
            assert_eq!(settings.output().dop().unwrap().rate(), DopRate::Dsd64);
        });
        settings.update(cx, |settings, cx| settings.select_dsd_quality(0, cx));
        cx.run_until_parked();

        assert_eq!(told.get(), before + 2);
        let saved = store.dsd_output().unwrap();
        assert_eq!(saved.rate, Some(DsdRate::Dsd64));
        assert_eq!(saved.quality, DsdQuality::Low);
    }

    #[gpui_kit::test]
    fn matching_the_song_family_is_off_until_asked_for(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let (settings, told, _subscription) = settings_saving_to(&store, cx);
        settings.update(cx, |settings, cx| {
            settings.set_device(DAC, cx);
            settings.turn_dsd_on(cx);
        });
        let frame_rates =
            |settings: &AudioSettings| settings.output().dop().unwrap().frame_rates_for(48_000)[0];
        settings.read_with(cx, |settings, _| {
            assert!(!settings.matches_song_family());
            assert_eq!(frame_rates(settings), 705_600);
        });
        let before = told.get();

        settings.update(cx, |settings, cx| {
            settings.set_matches_song_family(true, cx)
        });
        cx.run_until_parked();

        assert_eq!(told.get(), before + 1);
        assert!(store.dsd_output().unwrap().matches_song_family);
        settings.read_with(cx, |settings, _| {
            assert_eq!(frame_rates(settings), 768_000);
        });
    }

    #[gpui_kit::test]
    fn changing_a_value_makes_the_quality_custom_from_the_values_in_use(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let (settings, _told, _subscription) = settings_saving_to(&store, cx);

        settings.update(cx, |settings, cx| settings.select_dsd_quality(0, cx));
        settings.update(cx, |settings, cx| settings.set_dsd_noise_gain("1.3", cx));
        cx.run_until_parked();

        let parameters = store.dsd_output().unwrap().quality.parameters();
        assert!((parameters.noise_gain - 1.3).abs() < 1e-9);
        assert_eq!(parameters.order, DsdQuality::Low.parameters().order);
        settings.read_with(cx, |settings, _| assert_eq!(settings.dsd_quality_ix(), 4));
    }

    #[gpui_kit::test]
    fn choosing_custom_changes_nothing_until_a_value_does(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let (settings, _told, _subscription) = settings_saving_to(&store, cx);
        settings.update(cx, |settings, cx| settings.select_dsd_quality(1, cx));
        settings.update(cx, |settings, cx| settings.select_dsd_quality(4, cx));
        cx.run_until_parked();
        assert_eq!(
            store.dsd_output().unwrap().quality.parameters(),
            DsdQuality::Medium.parameters()
        );
    }

    #[gpui_kit::test]
    fn the_filter_keeps_its_window_and_each_kind_its_own_values(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let (settings, _told, _subscription) = settings_saving_to(&store, cx);
        settings.update(cx, |settings, cx| {
            settings.set_dsd_window("Hann²", cx);
            settings.set_dsd_filter(FFT, cx);
            settings.set_dsd_fft_size("8192", cx);
            settings.set_dsd_sinc_length("256", cx);
        });
        cx.run_until_parked();
        assert_eq!(
            store.dsd_output().unwrap().quality.parameters().filter,
            DsdFilter::Fft {
                size: 8192,
                window: ResamplingWindow::Hann2,
            }
        );
    }

    #[gpui_kit::test]
    fn off_values_come_back_as_none(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let (settings, _told, _subscription) = settings_saving_to(&store, cx);
        settings.update(cx, |settings, cx| {
            settings.set_dsd_dither("-100", cx);
            settings.set_dsd_limiter(OFF, cx);
        });
        cx.run_until_parked();
        let parameters = store.dsd_output().unwrap().quality.parameters();
        assert_eq!(parameters.dither_db, Some(-100.0));
        assert_eq!(parameters.limiter_ceiling_db, None);
    }

    #[test]
    fn a_rate_counts_when_either_family_fits() {
        assert!(is_rate_supported(&[192_000], DsdRate::Dsd64));
        assert!(is_rate_supported(&[176_400], DsdRate::Dsd64));
        assert!(!is_rate_supported(&[176_400, 192_000], DsdRate::Dsd128));
    }

    #[test]
    fn a_value_from_the_file_stays_an_option() {
        let label = |number: f64| format!("{number:.2}");
        assert_eq!(numbers(&[1.3, 1.4], 1.33, label).len(), 3);
        assert_eq!(numbers(&[1.3, 1.4], 1.4, label).len(), 2);
    }
}
