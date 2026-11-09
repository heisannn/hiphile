use audio::{CAN_PLAY_DOP, DopRate};
use config::{
    DSD_ORDERS, DSD_SINC_LENGTHS, DsdFilter, DsdParameters, DsdQuality, DsdRate,
    RESAMPLING_FFT_SIZES, ResamplingWindow,
};

use super::{AudioSettings, AudioSnapshot, MATCH_SONG, State, window_label};
use crate::settings::choices::{Choice, number_value, numbers, same_labels};

const DSD_QUALITIES: [&str; 5] = ["Low", "Medium", "High", "Max", "Custom"];

const PCM: &str = "pcm";
const DOP: &str = "dop";
const STANDARD_BASE_RATE: &str = "44100";
const PCM_ONLY: &str = "DSD plays songs whole while it's on.";

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

/// A setting for PCM, which DSD may turn off while it is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PcmSetting {
    SampleRate,
    ExclusiveMode,
    PlayPauseFade,
    TrimSilence,
    Crossfade,
    Resampling,
}

/// What to ask before a change that can play loud noise.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DsdWarning {
    title: String,
    message: &'static str,
    confirm_label: &'static str,
}

impl DsdWarning {
    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn message(&self) -> &str {
        self.message
    }

    /// For the choice that makes the change.
    pub fn confirm_label(&self) -> &str {
        self.confirm_label
    }
}

impl AudioSnapshot {
    pub fn is_dsd_on(&self) -> bool {
        self.output().dop().is_some()
    }

    /// While DSD is on, why `setting` has no effect.
    pub fn dsd_note(&self, setting: PcmSetting) -> Option<&'static str> {
        if !self.is_dsd_on() {
            return None;
        }
        Some(match setting {
            PcmSetting::SampleRate => "DSD sets the rate while it's on.",
            PcmSetting::ExclusiveMode => "DSD needs it while it's on.",
            PcmSetting::PlayPauseFade => "DSD cuts in and out at once while it's on.",
            PcmSetting::TrimSilence | PcmSetting::Crossfade => PCM_ONLY,
            PcmSetting::Resampling => "DSD uses the DSD filter settings below while it's on.",
        })
    }

    pub fn dsd_format_options(&self) -> Vec<Choice> {
        vec![Choice::new(PCM, "PCM"), Choice::new(DOP, "DSD (DoP)")]
    }

    pub fn dsd_format_value(&self) -> String {
        if self.is_dsd_on() { DOP } else { PCM }.to_owned()
    }

    /// What to ask before choosing `value` of [`Self::dsd_format_options`]. A device that cannot
    /// decode DoP plays it as noise at full level, whatever the app's volume was, and nothing
    /// the device reports can tell beforehand.
    pub fn dsd_format_warning(&self, value: &str) -> Option<DsdWarning> {
        (value == DOP && !self.is_dsd_on() && self.dsd_obstacle().is_none()).then(|| DsdWarning {
            title: format!("Turn on DSD for “{}”?", self.dsd_device_name()),
            message: "The app's volume stops working: sound comes at full level, so turn the \
                      device down first. A device without DoP support plays loud noise.",
            confirm_label: "Turn on",
        })
    }

    pub fn dsd_base_rate_options(&self) -> Vec<Choice> {
        vec![
            Choice::new(STANDARD_BASE_RATE, "44.1 kHz"),
            Choice::new(MATCH_SONG, "Match each song"),
        ]
    }

    pub fn dsd_base_rate_value(&self) -> String {
        if self.matches_song_family() {
            MATCH_SONG
        } else {
            STANDARD_BASE_RATE
        }
        .to_owned()
    }

    /// What to ask before choosing `value` of [`Self::dsd_base_rate_options`]. A DAC can decode
    /// the standard DSD rates and still take the 48 kHz family's for PCM, which a song already
    /// playing would turn into noise at once.
    pub fn dsd_base_rate_warning(&self, value: &str) -> Option<DsdWarning> {
        (value == MATCH_SONG && !self.matches_song_family()).then(|| DsdWarning {
            title: "Match each song's base rate?".to_owned(),
            message: "Songs at 48 kHz and its multiples then play at DSD rates based on 48 kHz. \
                      A DAC that doesn't decode those plays loud noise, so turn it down first.",
            confirm_label: "Match",
        })
    }

    /// Why DSD cannot be turned on for the chosen device.
    pub fn dsd_obstacle(&self) -> Option<&'static str> {
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

    pub fn dsd_device_name(&self) -> String {
        self.selected_device()
            .map_or("this device", |device| device.name())
            .to_owned()
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

    pub fn dsd_rate_options(&self) -> Vec<Choice> {
        DsdRate::ALL
            .into_iter()
            .map(|rate| {
                let name = format!("DSD{}", rate.multiple());
                let label = if self.has_dsd_rate(rate) {
                    name
                } else {
                    format!("{name} (not supported)")
                };
                Choice::new(rate.multiple().to_string(), label)
            })
            .collect()
    }

    pub fn dsd_rate_value(&self) -> String {
        self.dsd()
            .rate
            .unwrap_or(DsdRate::Dsd256)
            .multiple()
            .to_string()
    }

    pub fn matches_song_family(&self) -> bool {
        self.dsd().matches_song_family
    }

    pub fn fades_dsd_in(&self) -> bool {
        self.dsd().fades_in
    }

    pub fn dsd_quality_options(&self) -> Vec<Choice> {
        same_labels(DSD_QUALITIES.iter().map(|name| (*name).to_owned()))
    }

    pub fn dsd_quality_value(&self) -> String {
        DSD_QUALITIES[self.dsd_quality_ix()].to_owned()
    }

    pub fn dsd_quality_ix(&self) -> usize {
        match self.dsd().quality {
            DsdQuality::Low => 0,
            DsdQuality::Medium => 1,
            DsdQuality::High => 2,
            DsdQuality::Max => 3,
            DsdQuality::Custom(_) => 4,
        }
    }

    fn dsd_parameters(&self) -> DsdParameters {
        self.dsd().quality.parameters()
    }

    pub fn dsd_uses_sinc(&self) -> bool {
        matches!(self.dsd_parameters().filter, DsdFilter::Sinc { .. })
    }

    pub fn dsd_filter_options(&self) -> Vec<Choice> {
        vec![Choice::new(SINC, "Sinc"), Choice::new(FFT, "FFT")]
    }

    pub fn dsd_filter_value(&self) -> String {
        if self.dsd_uses_sinc() { SINC } else { FFT }.to_owned()
    }

    pub fn dsd_sinc_length_options(&self) -> Vec<Choice> {
        same_labels(DSD_SINC_LENGTHS.iter().map(u32::to_string))
    }

    pub fn dsd_sinc_length_value(&self) -> String {
        match self.dsd_parameters().filter {
            DsdFilter::Sinc { length, .. } => length,
            DsdFilter::Fft { .. } => DEFAULT_SINC_LENGTH,
        }
        .to_string()
    }

    pub fn dsd_fft_size_options(&self) -> Vec<Choice> {
        same_labels(RESAMPLING_FFT_SIZES.iter().map(u32::to_string))
    }

    pub fn dsd_fft_size_value(&self) -> String {
        match self.dsd_parameters().filter {
            DsdFilter::Fft { size, .. } => size,
            DsdFilter::Sinc { .. } => DEFAULT_FFT_SIZE,
        }
        .to_string()
    }

    pub fn dsd_window_value(&self) -> String {
        window_label(self.dsd_parameters().filter.window()).to_owned()
    }

    pub fn dsd_cutoff_options(&self) -> Vec<Choice> {
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

    pub fn dsd_cutoff_value(&self) -> String {
        self.dsd_cutoff().map_or(AUTO.to_owned(), number_value)
    }

    pub fn dsd_order_options(&self) -> Vec<Choice> {
        same_labels(DSD_ORDERS.iter().map(u8::to_string))
    }

    pub fn dsd_order_value(&self) -> String {
        self.dsd_parameters().order.to_string()
    }

    pub fn dsd_noise_gain_options(&self) -> Vec<Choice> {
        numbers(&NOISE_GAINS, self.dsd_parameters().noise_gain, |gain| {
            format!("{gain:.2}")
        })
    }

    pub fn dsd_noise_gain_value(&self) -> String {
        number_value(self.dsd_parameters().noise_gain)
    }

    pub fn dsd_noise_band_options(&self) -> Vec<Choice> {
        vec![
            Choice::new(NARROW_NOISE_BAND, "20 kHz"),
            Choice::new(WIDE_NOISE_BAND, "48 kHz"),
        ]
    }

    pub fn dsd_noise_band_value(&self) -> String {
        if self.dsd_parameters().wide_noise_band && self.is_dsd_noise_band_widenable() {
            WIDE_NOISE_BAND
        } else {
            NARROW_NOISE_BAND
        }
        .to_owned()
    }

    /// Only DSD256 takes the wide noise band.
    pub fn is_dsd_noise_band_widenable(&self) -> bool {
        self.dsd().rate.unwrap_or(DsdRate::Dsd256) == DsdRate::Dsd256
    }

    pub fn has_optimized_dsd_zeros(&self) -> bool {
        self.dsd_parameters().optimized_zeros
    }

    pub fn dsd_dither_options(&self) -> Vec<Choice> {
        numbers_or(
            OFF,
            "Off",
            &DITHER_DBS,
            self.dsd_parameters().dither_db,
            |db| format!("{db:.0} dB"),
        )
    }

    pub fn dsd_dither_value(&self) -> String {
        self.dsd_parameters()
            .dither_db
            .map_or(OFF.to_owned(), number_value)
    }

    pub fn dsd_input_level_options(&self) -> Vec<Choice> {
        let current = self.dsd_parameters().input_level_db;
        numbers(&INPUT_LEVEL_DBS, current, |db| format!("{db:+.0} dB"))
    }

    pub fn dsd_input_level_value(&self) -> String {
        number_value(self.dsd_parameters().input_level_db)
    }

    pub fn dsd_limiter_options(&self) -> Vec<Choice> {
        let current = self.dsd_parameters().limiter_ceiling_db;
        numbers_or(OFF, "Off", &LIMITER_CEILING_DBS, current, |db| {
            format!("{db:+.0} dBTP")
        })
    }

    pub fn dsd_limiter_value(&self) -> String {
        self.dsd_parameters()
            .limiter_ceiling_db
            .map_or(OFF.to_owned(), number_value)
    }
}

impl AudioSettings {
    pub fn is_dsd_offered() -> bool {
        CAN_PLAY_DOP
    }

    /// Asks for no confirmation itself: [`AudioSnapshot::dsd_format_warning`] says what to ask.
    /// Does nothing while there is an [`AudioSnapshot::dsd_obstacle`].
    pub fn turn_dsd_on(&self) {
        self.change(State::turn_dsd_on);
    }

    pub fn turn_dsd_off(&self) {
        self.change(|state| state.change_dsd(|dsd| dsd.is_on = false));
    }

    /// `value` is one of [`AudioSnapshot::dsd_rate_options`].
    pub fn set_dsd_rate(&self, value: &str) {
        self.change(|state| state.set_dsd_rate(value));
    }

    /// `value` is one of [`AudioSnapshot::dsd_format_options`]. DSD turns on as
    /// [`Self::turn_dsd_on`] turns it on.
    pub fn set_dsd_format(&self, value: &str) {
        match value {
            DOP => self.turn_dsd_on(),
            PCM => self.turn_dsd_off(),
            _ => {}
        }
    }

    /// Asks for no confirmation itself: [`AudioSnapshot::dsd_base_rate_warning`] says what to
    /// ask.
    pub fn set_matches_song_family(&self, matches: bool) {
        self.change(|state| state.change_dsd(|dsd| dsd.matches_song_family = matches));
    }

    /// `value` is one of [`AudioSnapshot::dsd_base_rate_options`].
    pub fn set_dsd_base_rate(&self, value: &str) {
        match value {
            MATCH_SONG => self.set_matches_song_family(true),
            STANDARD_BASE_RATE => self.set_matches_song_family(false),
            _ => {}
        }
    }

    pub fn set_fades_dsd_in(&self, is_on: bool) {
        self.change(|state| state.change_dsd(|dsd| dsd.fades_in = is_on));
    }

    /// `value` is one of [`AudioSnapshot::dsd_quality_options`].
    pub fn set_dsd_quality(&self, value: &str) {
        if let Some(ix) = DSD_QUALITIES.iter().position(|name| *name == value) {
            self.select_dsd_quality(ix);
        }
    }

    /// `ix` is into the qualities of [`AudioSnapshot::dsd_quality_options`]. Custom starts from
    /// the values in use, so choosing it changes nothing until one of them is changed.
    pub fn select_dsd_quality(&self, ix: usize) {
        self.change(|state| state.select_dsd_quality(ix));
    }

    /// `value` is one of [`AudioSnapshot::dsd_filter_options`].
    pub fn set_dsd_filter(&self, value: &str) {
        self.change(|state| state.set_dsd_filter(value));
    }

    pub fn set_dsd_sinc_length(&self, value: &str) {
        let Ok(length) = value.parse() else {
            return;
        };
        self.change_dsd_parameters(|parameters| {
            if let DsdFilter::Sinc {
                length: current, ..
            } = &mut parameters.filter
            {
                *current = length;
            }
        });
    }

    pub fn set_dsd_fft_size(&self, value: &str) {
        let Ok(size) = value.parse() else {
            return;
        };
        self.change_dsd_parameters(|parameters| {
            if let DsdFilter::Fft { size: current, .. } = &mut parameters.filter {
                *current = size;
            }
        });
    }

    /// `value` is one of [`AudioSnapshot::window_options`].
    pub fn set_dsd_window(&self, value: &str) {
        let window = ResamplingWindow::ALL
            .into_iter()
            .find(|window| window_label(*window) == value);
        if let Some(window) = window {
            self.change_dsd_parameters(|parameters| {
                parameters.filter = parameters.filter.with_window(window);
            });
        }
    }

    pub fn set_dsd_cutoff(&self, value: &str) {
        let cutoff = value.parse().ok();
        self.change_dsd_parameters(|parameters| {
            if let DsdFilter::Sinc {
                cutoff: current, ..
            } = &mut parameters.filter
            {
                *current = cutoff;
            }
        });
    }

    pub fn set_dsd_order(&self, value: &str) {
        if let Ok(order) = value.parse() {
            self.change_dsd_parameters(|parameters| parameters.order = order);
        }
    }

    pub fn set_dsd_noise_gain(&self, value: &str) {
        if let Ok(noise_gain) = value.parse() {
            self.change_dsd_parameters(|parameters| parameters.noise_gain = noise_gain);
        }
    }

    pub fn set_dsd_noise_band(&self, value: &str) {
        self.change_dsd_parameters(|parameters| {
            parameters.wide_noise_band = value == WIDE_NOISE_BAND;
        });
    }

    pub fn set_optimized_dsd_zeros(&self, is_on: bool) {
        self.change_dsd_parameters(|parameters| parameters.optimized_zeros = is_on);
    }

    pub fn set_dsd_dither(&self, value: &str) {
        let dither_db = value.parse().ok();
        self.change_dsd_parameters(|parameters| parameters.dither_db = dither_db);
    }

    pub fn set_dsd_input_level(&self, value: &str) {
        if let Ok(input_level_db) = value.parse() {
            self.change_dsd_parameters(|parameters| parameters.input_level_db = input_level_db);
        }
    }

    pub fn set_dsd_limiter(&self, value: &str) {
        let ceiling = value.parse().ok();
        self.change_dsd_parameters(|parameters| parameters.limiter_ceiling_db = ceiling);
    }

    /// A change makes the quality custom.
    fn change_dsd_parameters(&self, change: impl FnOnce(&mut DsdParameters)) {
        self.change(|state| state.change_dsd_parameters(change));
    }
}

impl State {
    fn turn_dsd_on(&mut self) {
        let settings = &self.settings;
        if settings.dsd_obstacle().is_some() {
            return;
        }
        let mut dsd = settings.dsd().clone();
        dsd.is_on = true;
        dsd.device = settings.output().device().map(str::to_owned);
        dsd.rate = Some(
            dsd.rate
                .filter(|rate| settings.has_dsd_rate(*rate))
                .unwrap_or_else(|| settings.highest_dsd_rate()),
        );
        self.set_dsd(dsd);
    }

    fn change_dsd(&mut self, change: impl FnOnce(&mut config::DsdOutput)) {
        let mut dsd = self.settings.dsd().clone();
        change(&mut dsd);
        self.set_dsd(dsd);
    }

    fn set_dsd_rate(&mut self, value: &str) {
        let rate = DsdRate::ALL
            .into_iter()
            .find(|rate| rate.multiple().to_string() == value);
        if rate.is_some() {
            self.change_dsd(|dsd| dsd.rate = rate);
        }
    }

    fn select_dsd_quality(&mut self, ix: usize) {
        self.change_dsd(|dsd| {
            dsd.quality = match ix {
                0 => DsdQuality::Low,
                1 => DsdQuality::Medium,
                2 => DsdQuality::High,
                3 => DsdQuality::Max,
                _ => DsdQuality::Custom(dsd.quality.parameters()),
            };
        });
    }

    fn change_dsd_parameters(&mut self, change: impl FnOnce(&mut DsdParameters)) {
        let mut parameters = self.settings.dsd_parameters();
        change(&mut parameters);
        if parameters != self.settings.dsd_parameters() {
            self.change_dsd(|dsd| dsd.quality = DsdQuality::Custom(parameters));
        }
    }

    fn set_dsd_filter(&mut self, value: &str) {
        if self.settings.dsd_filter_value() == value {
            return;
        }
        let window = self.settings.dsd_parameters().filter.window();
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
        self.change_dsd_parameters(|parameters| parameters.filter = filter);
    }
}

/// A DSD rate counts when either family's frame rate fits, as the other family stands in for the
/// one a device lacks.
fn is_rate_supported(frame_rates: &[u32], rate: DsdRate) -> bool {
    let rate = match rate {
        DsdRate::Dsd64 => DopRate::Dsd64,
        DsdRate::Dsd128 => DopRate::Dsd128,
        DsdRate::Dsd256 => DopRate::Dsd256,
    };
    frame_rates.contains(&rate.frame_rate_for(44_100))
        || frame_rates.contains(&rate.frame_rate_for(48_000))
}

fn numbers_or(
    none_value: &'static str,
    none_label: &'static str,
    values: &[f64],
    current: Option<f64>,
    label: impl Fn(f64) -> String,
) -> Vec<Choice> {
    let mut options = vec![Choice::new(none_value, none_label)];
    match current {
        Some(current) => options.extend(numbers(values, current, label)),
        None => options.extend(
            values
                .iter()
                .map(|number| Choice::new(number_value(*number), label(*number))),
        ),
    }
    options
}

#[cfg(test)]
mod tests {
    use gpui_kit::TestAppContext;

    use super::*;
    use crate::settings::audio::tests::{DAC, dac, settings_saving_to, told};

    /// With the DAC listed and chosen.
    fn settings_on_the_dac(store: &config::Store, cx: &mut TestAppContext) -> AudioSettings {
        let settings = settings_saving_to(store, cx);
        settings.pretend_listed(vec![dac()]);
        settings.set_device(DAC);
        settings
    }

    #[gpui_kit::test]
    fn dsd_goes_on_for_the_chosen_device_and_is_saved(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let settings = settings_on_the_dac(&store, cx);

        settings.turn_dsd_on();
        cx.run_until_parked();

        assert_eq!(told(&settings), 2);
        let saved = store.dsd_output().unwrap();
        assert!(saved.is_on);
        assert_eq!(saved.device.as_deref(), Some(DAC));
        assert_eq!(saved.rate, Some(DsdRate::Dsd256));
        let settings = settings.snapshot();
        assert!(settings.is_dsd_on());
        assert!(settings.output().dop().is_some());
    }

    #[gpui_kit::test]
    fn another_device_plays_pcm_until_dsd_is_turned_on_for_it(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let settings = settings_on_the_dac(&store, cx);
        settings.turn_dsd_on();
        settings.set_device("coreaudio:other");
        assert!(!settings.snapshot().is_dsd_on());

        settings.set_device(DAC);
        assert!(settings.snapshot().is_dsd_on());
    }

    #[gpui_kit::test]
    fn the_fade_can_be_turned_off_and_reaches_the_output(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let settings = settings_on_the_dac(&store, cx);
        settings.turn_dsd_on();
        let snapshot = settings.snapshot();
        assert!(snapshot.fades_dsd_in());
        assert!(snapshot.output().dop_fades_in());
        let before = told(&settings);

        settings.set_fades_dsd_in(false);
        cx.run_until_parked();

        assert_eq!(told(&settings), before + 1);
        assert!(!store.dsd_output().unwrap().fades_in);
        let settings = settings.snapshot();
        assert!(!settings.fades_dsd_in());
        assert!(!settings.output().dop_fades_in());
    }

    #[gpui_kit::test]
    fn max_reaches_the_output_and_stays_selected(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let settings = settings_saving_to(&store, cx);
        settings.set_dsd_quality("Max");
        cx.run_until_parked();
        assert_eq!(store.dsd_output().unwrap().quality, DsdQuality::Max);
        let settings = settings.snapshot();
        assert_eq!(settings.dsd_quality_value(), "Max");
        assert_eq!(settings.dsd_sinc_length_value(), "2048");
    }

    #[gpui_kit::test]
    fn a_chosen_cutoff_is_listed_once(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let settings = settings_on_the_dac(&store, cx);
        settings.turn_dsd_on();
        settings.set_dsd_cutoff("0.85");
        let settings = settings.snapshot();
        assert_eq!(settings.dsd_cutoff_value(), "0.85");
        assert_eq!(settings.dsd_cutoff_options().len(), CUTOFFS.len() + 1);
    }

    #[gpui_kit::test]
    fn the_wide_noise_band_shows_only_where_it_applies(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let settings = settings_on_the_dac(&store, cx);
        settings.turn_dsd_on();
        settings.set_dsd_noise_band(WIDE_NOISE_BAND);
        assert_eq!(settings.snapshot().dsd_noise_band_value(), WIDE_NOISE_BAND);
        settings.set_dsd_rate("64");
        assert_eq!(
            settings.snapshot().dsd_noise_band_value(),
            NARROW_NOISE_BAND
        );
    }

    #[gpui_kit::test]
    fn the_system_default_can_have_no_dsd(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let settings = settings_saving_to(&store, cx);
        assert!(settings.snapshot().dsd_obstacle().is_some());
        settings.turn_dsd_on();
        assert!(!settings.snapshot().is_dsd_on());
    }

    #[gpui_kit::test]
    fn a_new_rate_or_preset_reaches_the_output(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let settings = settings_on_the_dac(&store, cx);
        settings.turn_dsd_on();
        let before = told(&settings);

        settings.set_dsd_rate("64");
        assert_eq!(
            settings.snapshot().output().dop().unwrap().rate(),
            DopRate::Dsd64
        );
        settings.select_dsd_quality(0);
        cx.run_until_parked();

        assert_eq!(told(&settings), before + 2);
        let saved = store.dsd_output().unwrap();
        assert_eq!(saved.rate, Some(DsdRate::Dsd64));
        assert_eq!(saved.quality, DsdQuality::Low);
    }

    #[gpui_kit::test]
    fn matching_the_song_family_is_off_until_asked_for(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let settings = settings_on_the_dac(&store, cx);
        settings.turn_dsd_on();
        let frame_rates =
            |settings: &AudioSnapshot| settings.output().dop().unwrap().frame_rates_for(48_000)[0];
        let snapshot = settings.snapshot();
        assert!(!snapshot.matches_song_family());
        assert_eq!(frame_rates(&snapshot), 705_600);
        let before = told(&settings);

        settings.set_matches_song_family(true);
        cx.run_until_parked();

        assert_eq!(told(&settings), before + 1);
        assert!(store.dsd_output().unwrap().matches_song_family);
        assert_eq!(frame_rates(&settings.snapshot()), 768_000);
    }

    #[gpui_kit::test]
    fn changing_a_value_makes_the_quality_custom_from_the_values_in_use(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let settings = settings_saving_to(&store, cx);

        settings.select_dsd_quality(0);
        settings.set_dsd_noise_gain("1.3");
        cx.run_until_parked();

        let parameters = store.dsd_output().unwrap().quality.parameters();
        assert!((parameters.noise_gain - 1.3).abs() < 1e-9);
        assert_eq!(parameters.order, DsdQuality::Low.parameters().order);
        assert_eq!(settings.snapshot().dsd_quality_ix(), 4);
    }

    #[gpui_kit::test]
    fn choosing_custom_changes_nothing_until_a_value_does(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let settings = settings_saving_to(&store, cx);
        settings.select_dsd_quality(1);
        settings.select_dsd_quality(4);
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
        let settings = settings_saving_to(&store, cx);
        settings.set_dsd_window("Hann²");
        settings.set_dsd_filter(FFT);
        settings.set_dsd_fft_size("8192");
        settings.set_dsd_sinc_length("256");
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
        let settings = settings_saving_to(&store, cx);
        settings.set_dsd_dither("-100");
        settings.set_dsd_limiter(OFF);
        cx.run_until_parked();
        let parameters = store.dsd_output().unwrap().quality.parameters();
        assert_eq!(parameters.dither_db, Some(-100.0));
        assert_eq!(parameters.limiter_ceiling_db, None);
    }

    #[gpui_kit::test]
    fn dsd_stays_off_for_a_device_that_cannot_take_it(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let settings = settings_saving_to(&store, cx);
        settings.set_device(DAC);
        settings.turn_dsd_on();
        assert!(!settings.snapshot().is_dsd_on());

        let speakers = audio::OutputDevice::listed(DAC, "Speakers", &[44_100, 48_000], &[]);
        settings.pretend_listed(vec![speakers]);
        assert!(settings.snapshot().dsd_obstacle().is_some());
        settings.set_dsd_format(DOP);
        cx.run_until_parked();

        assert!(!settings.snapshot().is_dsd_on());
        assert!(!store.dsd_output().unwrap().is_on);
    }

    #[gpui_kit::test]
    fn turning_dsd_on_asks_first_in_the_device_s_name(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let settings = settings_on_the_dac(&store, cx);
        let snapshot = settings.snapshot();
        assert_eq!(snapshot.dsd_format_warning(PCM), None);
        let warning = snapshot.dsd_format_warning(DOP).unwrap();
        assert_eq!(warning.title(), "Turn on DSD for “DAC”?");
        assert_eq!(warning.confirm_label(), "Turn on");

        settings.set_dsd_format(DOP);

        let snapshot = settings.snapshot();
        assert_eq!(snapshot.dsd_format_value(), DOP);
        assert_eq!(snapshot.dsd_format_warning(DOP), None);
        settings.set_dsd_format(PCM);
        assert_eq!(settings.snapshot().dsd_format_value(), PCM);
    }

    #[gpui_kit::test]
    fn without_dsd_there_is_nothing_to_ask(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let settings = settings_saving_to(&store, cx);
        assert_eq!(settings.snapshot().dsd_format_warning(DOP), None);
    }

    #[gpui_kit::test]
    fn only_matching_the_song_family_asks_first(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let settings = settings_on_the_dac(&store, cx);
        settings.turn_dsd_on();
        let snapshot = settings.snapshot();
        assert_eq!(snapshot.dsd_base_rate_value(), STANDARD_BASE_RATE);
        assert_eq!(snapshot.dsd_base_rate_warning(STANDARD_BASE_RATE), None);
        let warning = snapshot.dsd_base_rate_warning(MATCH_SONG).unwrap();
        assert_eq!(warning.confirm_label(), "Match");

        settings.set_dsd_base_rate(MATCH_SONG);
        let snapshot = settings.snapshot();
        assert!(snapshot.matches_song_family());
        assert_eq!(snapshot.dsd_base_rate_warning(MATCH_SONG), None);

        settings.set_dsd_base_rate(STANDARD_BASE_RATE);
        assert!(!settings.snapshot().matches_song_family());
    }

    #[gpui_kit::test]
    fn dsd_says_why_the_pcm_settings_are_off_only_while_it_is_on(cx: &mut TestAppContext) {
        let all = [
            PcmSetting::SampleRate,
            PcmSetting::ExclusiveMode,
            PcmSetting::PlayPauseFade,
            PcmSetting::TrimSilence,
            PcmSetting::Crossfade,
            PcmSetting::Resampling,
        ];
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let settings = settings_on_the_dac(&store, cx);
        assert!(
            all.iter()
                .all(|pcm| settings.snapshot().dsd_note(*pcm).is_none())
        );

        settings.turn_dsd_on();

        assert!(
            all.iter()
                .all(|pcm| settings.snapshot().dsd_note(*pcm).is_some())
        );
    }

    #[test]
    fn a_rate_counts_when_either_family_fits() {
        assert!(is_rate_supported(&[192_000], DsdRate::Dsd64));
        assert!(is_rate_supported(&[176_400], DsdRate::Dsd64));
        assert!(!is_rate_supported(&[176_400, 192_000], DsdRate::Dsd128));
    }
}
