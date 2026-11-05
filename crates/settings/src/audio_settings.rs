use audio::{OutputDevice, OutputSettings, Resampling};
use config::{RESAMPLING_FFT_SIZES, ResamplingQuality, ResamplingWindow};
use gpui_kit::{AppContext as _, Context, EventEmitter, SharedString, Task};
use ui::format_sample_rate;

/// Device ids hold a colon, so none is equal to this.
pub(crate) const SYSTEM_DEFAULT: &str = "system-default";
pub(crate) const MATCH_SONG: &str = "match-song";
pub(crate) const QUALITIES: [&str; 4] = ["Medium", "High", "Max", "Custom"];

pub struct OutputChanged;

pub struct AudioSettings {
    /// `None` without a home directory: choices then last until quitting.
    store: Option<config::Store>,
    output: OutputSettings,
    quality: ResamplingQuality,
    /// `None` until first listed.
    devices: Option<Vec<OutputDevice>>,
    list_error: Option<SharedString>,
    save_error: Option<SharedString>,
    listing: bool,
    _list: Task<()>,
    /// Which of `output`'s values the user has changed since they were last
    /// saved. Only those are written: the rest may have come from the
    /// environment, or from a file that could not be read.
    unsaved: Unsaved,
    saving: bool,
    _save: Task<()>,
}

#[derive(Clone, Copy, Default)]
struct Unsaved {
    device: bool,
    rate: bool,
    quality: bool,
}

impl EventEmitter<OutputChanged> for AudioSettings {}

impl AudioSettings {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let (store, output, quality, save_error) = read_saved_output();
        let mut this = Self::with(store, output, quality, save_error);
        this.list_devices(cx);
        this
    }

    fn with(
        store: Option<config::Store>,
        output: OutputSettings,
        quality: ResamplingQuality,
        save_error: Option<SharedString>,
    ) -> Self {
        Self {
            store,
            output,
            quality,
            devices: None,
            list_error: None,
            save_error,
            listing: false,
            _list: Task::ready(()),
            unsaved: Unsaved::default(),
            saving: false,
            _save: Task::ready(()),
        }
    }

    pub fn output(&self) -> &OutputSettings {
        &self.output
    }

    /// Lists the devices again, unless a listing is still in progress:
    /// replacing it could keep a slow system from ever finishing one.
    pub(crate) fn list_devices(&mut self, cx: &mut Context<Self>) {
        if self.listing {
            return;
        }
        self.listing = true;
        self._list = cx.spawn(async move |this, cx| {
            let devices = cx.background_spawn(async { audio::output_devices() }).await;
            this.update(cx, |this, cx| this.on_devices_listed(devices, cx))
                .ok();
        });
    }

    fn on_devices_listed(
        &mut self,
        devices: Result<Vec<OutputDevice>, audio::Error>,
        cx: &mut Context<Self>,
    ) {
        self.listing = false;
        let (devices, list_error) = match devices {
            Ok(devices) => (Some(devices), None),
            Err(error) => (
                self.devices.clone(),
                Some(format!("Couldn't list the output devices: {error}.").into()),
            ),
        };
        if self.devices != devices || self.list_error != list_error {
            self.devices = devices;
            self.list_error = list_error;
            cx.notify();
        }
    }

    pub(crate) fn has_devices(&self) -> bool {
        self.devices.is_some()
    }

    pub(crate) fn errors(&self) -> impl Iterator<Item = &SharedString> {
        self.list_error.iter().chain(&self.save_error)
    }

    pub(crate) fn device_options(&self) -> Vec<(SharedString, SharedString)> {
        let devices = self
            .devices
            .iter()
            .flatten()
            .filter_map(|device| device.id().map(|id| (id, device.name())));
        device_options(devices, self.output.device(), self.has_devices())
    }

    pub(crate) fn device_value(&self) -> SharedString {
        self.output
            .device()
            .map_or(SYSTEM_DEFAULT.into(), |id| id.to_owned().into())
    }

    pub(crate) fn set_device(&mut self, value: &str, cx: &mut Context<Self>) {
        let device = (value != SYSTEM_DEFAULT).then(|| value.to_owned());
        let output = OutputSettings::new(device, self.output.fixed_rate())
            .with_resampling(self.output.resampling());
        if self.output != output {
            self.unsaved.device = true;
            self.set_output(output, cx);
        }
    }

    pub(crate) fn rate_options(&self) -> Vec<(SharedString, SharedString)> {
        rate_options(self.selected_device_rates(), self.output.fixed_rate())
    }

    pub(crate) fn rate_value(&self) -> SharedString {
        self.output
            .fixed_rate()
            .map_or(MATCH_SONG.into(), |rate| rate.to_string().into())
    }

    pub(crate) fn set_rate(&mut self, value: &str, cx: &mut Context<Self>) {
        let device = self.output.device().map(str::to_owned);
        let output = OutputSettings::new(device, value.parse().ok())
            .with_resampling(self.output.resampling());
        if self.output != output {
            self.unsaved.rate = true;
            self.set_output(output, cx);
        }
    }

    pub(crate) fn quality_ix(&self) -> usize {
        match self.quality {
            ResamplingQuality::Medium => 0,
            ResamplingQuality::High => 1,
            ResamplingQuality::Max => 2,
            ResamplingQuality::Custom { .. } => 3,
        }
    }

    /// `ix` is into [`QUALITIES`]. Custom starts from the values in use, so
    /// choosing it changes nothing until one of them is changed.
    pub(crate) fn select_quality(&mut self, ix: usize, cx: &mut Context<Self>) {
        let quality = match ix {
            0 => ResamplingQuality::Medium,
            1 => ResamplingQuality::High,
            2 => ResamplingQuality::Max,
            _ => ResamplingQuality::Custom {
                fft_size: self.quality.fft_size(),
                window: self.quality.window(),
            },
        };
        self.set_quality(quality, cx);
    }

    pub(crate) fn fft_size_options(&self) -> Vec<(SharedString, SharedString)> {
        RESAMPLING_FFT_SIZES
            .iter()
            .map(|size| {
                let size = SharedString::from(size.to_string());
                (size.clone(), size)
            })
            .collect()
    }

    pub(crate) fn fft_size_value(&self) -> SharedString {
        self.quality.fft_size().to_string().into()
    }

    /// Another size than the one in use makes the quality custom.
    pub(crate) fn set_fft_size(&mut self, value: &str, cx: &mut Context<Self>) {
        let Ok(fft_size) = value.parse() else {
            return;
        };
        if fft_size != self.quality.fft_size() {
            let window = self.quality.window();
            self.set_quality(ResamplingQuality::Custom { fft_size, window }, cx);
        }
    }

    pub(crate) fn window_options(&self) -> Vec<(SharedString, SharedString)> {
        ResamplingWindow::ALL
            .iter()
            .map(|window| (window_label(*window).into(), window_label(*window).into()))
            .collect()
    }

    pub(crate) fn window_value(&self) -> SharedString {
        window_label(self.quality.window()).into()
    }

    /// Another window than the one in use makes the quality custom.
    pub(crate) fn set_window(&mut self, value: &str, cx: &mut Context<Self>) {
        let window = ResamplingWindow::ALL
            .into_iter()
            .find(|window| window_label(*window) == value);
        if let Some(window) = window.filter(|window| *window != self.quality.window()) {
            let fft_size = self.quality.fft_size();
            self.set_quality(ResamplingQuality::Custom { fft_size, window }, cx);
        }
    }

    fn set_quality(&mut self, quality: ResamplingQuality, cx: &mut Context<Self>) {
        if self.quality == quality {
            return;
        }
        self.quality = quality;
        self.unsaved.quality = true;
        let output = self.output.clone().with_resampling(resampling_of(quality));
        if self.output == output {
            self.save(cx);
            cx.notify();
        } else {
            self.set_output(output, cx);
        }
    }

    fn selected_device_rates(&self) -> Option<&[u32]> {
        let mut devices = self.devices.iter().flatten();
        let device = match self.output.device() {
            Some(id) => devices.find(|device| device.id() == Some(id)),
            None => devices.find(|device| device.is_default()),
        };
        device.map(OutputDevice::sample_rates)
    }

    fn set_output(&mut self, output: OutputSettings, cx: &mut Context<Self>) {
        self.output = output;
        self.save(cx);
        cx.emit(OutputChanged);
        cx.notify();
    }

    /// One write at a time, so a slow one cannot leave an older choice on
    /// disk; the latest is written when the current write finishes.
    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        if self.saving {
            return;
        }
        self.saving = true;
        self._save = cx.spawn(async move |this, cx| {
            while let Ok(Some((output, quality, unsaved))) = this.update(cx, |this, _| {
                let unsaved = std::mem::take(&mut this.unsaved);
                this.saving = unsaved.device || unsaved.rate || unsaved.quality;
                this.saving
                    .then(|| (this.output.clone(), this.quality, unsaved))
            }) {
                let store = store.clone();
                let saved = cx
                    .background_spawn(async move {
                        if unsaved.device {
                            store.save_output_device(output.device())?;
                        }
                        if unsaved.rate {
                            store.save_sample_rate(output.fixed_rate())?;
                        }
                        if unsaved.quality {
                            store.save_resampling_quality(quality)?;
                        }
                        Ok::<_, config::Error>(())
                    })
                    .await;
                this.update(cx, |this, cx| {
                    this.save_error = saved
                        .err()
                        .map(|error| format!("Couldn't save the audio settings: {error}.").into());
                    cx.notify();
                })
                .ok();
            }
        });
    }
}

/// The saved output, and why it could not be read or will not be saved.
fn read_saved_output() -> (
    Option<config::Store>,
    OutputSettings,
    ResamplingQuality,
    Option<SharedString>,
) {
    let store = match config::Store::user() {
        Ok(store) => store,
        Err(error) => {
            let error = format!("Audio settings won't be saved: {error}.").into();
            let quality = ResamplingQuality::default();
            return (None, output_with(None, None, quality), quality, Some(error));
        }
    };
    match (
        store.output_device(),
        store.sample_rate(),
        store.resampling_quality(),
    ) {
        (Ok(device), Ok(rate), Ok(quality)) => (
            Some(store),
            output_with(device, rate, quality),
            quality,
            None,
        ),
        (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
            let error = format!("Couldn't read the audio settings: {error}.").into();
            let quality = ResamplingQuality::default();
            (
                Some(store),
                output_with(None, None, quality),
                quality,
                Some(error),
            )
        }
    }
}

fn output_with(
    device: Option<String>,
    rate: Option<u32>,
    quality: ResamplingQuality,
) -> OutputSettings {
    OutputSettings::new(device, rate).with_resampling(resampling_of(quality))
}

fn resampling_of(quality: ResamplingQuality) -> Resampling {
    let window = match quality.window() {
        ResamplingWindow::Hann => audio::ResamplingWindow::Hann,
        ResamplingWindow::Hann2 => audio::ResamplingWindow::Hann2,
        ResamplingWindow::Blackman => audio::ResamplingWindow::Blackman,
        ResamplingWindow::Blackman2 => audio::ResamplingWindow::Blackman2,
        ResamplingWindow::BlackmanHarris => audio::ResamplingWindow::BlackmanHarris,
        ResamplingWindow::BlackmanHarris2 => audio::ResamplingWindow::BlackmanHarris2,
    };
    Resampling::new(quality.fft_size(), window)
}

fn window_label(window: ResamplingWindow) -> &'static str {
    match window {
        ResamplingWindow::Hann => "Hann",
        ResamplingWindow::Hann2 => "Hann²",
        ResamplingWindow::Blackman => "Blackman",
        ResamplingWindow::Blackman2 => "Blackman²",
        ResamplingWindow::BlackmanHarris => "Blackman-Harris",
        ResamplingWindow::BlackmanHarris2 => "Blackman-Harris²",
    }
}

/// `devices` are `(id, name)`. A chosen device that is not among them stays
/// an option, so the choice stays visible until it is connected again.
fn device_options<'a>(
    devices: impl Iterator<Item = (&'a str, &'a str)>,
    selected: Option<&str>,
    listed: bool,
) -> Vec<(SharedString, SharedString)> {
    let mut options: Vec<(SharedString, SharedString)> =
        vec![(SYSTEM_DEFAULT.into(), "System default".into())];
    options.extend(devices.map(|(id, name)| (id.to_owned().into(), name.to_owned().into())));
    if let Some(selected) = selected
        && !options.iter().any(|(id, _)| id == selected)
    {
        let label = if listed {
            "Disconnected device"
        } else {
            "Loading"
        };
        options.push((selected.to_owned().into(), label.into()));
    }
    options
}

/// `rates` are what the device supports, if it is listed. A fixed rate that
/// is not among them stays an option; the engine keeps the device's own rate
/// meanwhile. Without a listed device, nothing is known to be unsupported.
fn rate_options(rates: Option<&[u32]>, fixed: Option<u32>) -> Vec<(SharedString, SharedString)> {
    let mut options: Vec<(SharedString, SharedString)> =
        vec![(MATCH_SONG.into(), "Match each song".into())];
    let rate_option = |rate: u32, label: String| (rate.to_string().into(), label.into());
    options.extend(
        rates
            .unwrap_or_default()
            .iter()
            .map(|rate| rate_option(*rate, format_sample_rate(*rate))),
    );
    if let Some(fixed) = fixed.filter(|fixed| !rates.unwrap_or_default().contains(fixed)) {
        let label = match rates {
            Some(_) => format!("{} (not supported)", format_sample_rate(fixed)),
            None => format_sample_rate(fixed),
        };
        options.push(rate_option(fixed, label));
    }
    options
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use gpui_kit::{Entity, TestAppContext};

    use super::*;

    /// `AudioSettings::new` would list the machine's devices.
    fn settings_saving_to(
        store: &config::Store,
        cx: &mut TestAppContext,
    ) -> (Entity<AudioSettings>, Rc<Cell<u32>>, gpui_kit::Subscription) {
        let quality = ResamplingQuality::default();
        let settings = cx.new(|_| {
            AudioSettings::with(
                Some(store.clone()),
                output_with(None, None, quality),
                quality,
                None,
            )
        });
        let told = Rc::new(Cell::new(0));
        let subscription = cx.update(|cx| {
            let told = told.clone();
            cx.subscribe(&settings, move |_, _: &OutputChanged, _| {
                told.set(told.get() + 1);
            })
        });
        (settings, told, subscription)
    }

    #[gpui_kit::test]
    fn a_preset_is_told_saved_and_reaches_the_output(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let (settings, told, _subscription) = settings_saving_to(&store, cx);

        settings.update(cx, |settings, cx| settings.select_quality(2, cx));
        cx.run_until_parked();

        assert_eq!(told.get(), 1);
        assert_eq!(store.resampling_quality().unwrap(), ResamplingQuality::Max);
        settings.read_with(cx, |settings, _| {
            assert_eq!(settings.quality_ix(), 2);
            assert_eq!(settings.output().resampling().fft_size(), 4096);
        });
    }

    #[gpui_kit::test]
    fn choosing_custom_is_saved_but_keeps_the_sound(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let (settings, told, _subscription) = settings_saving_to(&store, cx);
        settings.update(cx, |settings, cx| settings.select_quality(1, cx));
        cx.run_until_parked();
        assert_eq!(told.get(), 1);

        settings.update(cx, |settings, cx| settings.select_quality(3, cx));
        cx.run_until_parked();

        assert_eq!(told.get(), 1);
        let custom = ResamplingQuality::Custom {
            fft_size: 1024,
            window: ResamplingWindow::BlackmanHarris2,
        };
        assert_eq!(store.resampling_quality().unwrap(), custom);
        settings.read_with(cx, |settings, _| assert_eq!(settings.quality_ix(), 3));
    }

    #[gpui_kit::test]
    fn another_value_turns_a_preset_custom_and_the_same_one_does_not(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let (settings, told, _subscription) = settings_saving_to(&store, cx);
        settings.update(cx, |settings, cx| settings.select_quality(1, cx));
        cx.run_until_parked();
        assert_eq!(told.get(), 1);

        settings.update(cx, |settings, cx| {
            settings.set_fft_size("1024", cx);
            settings.set_window("Blackman-Harris²", cx);
        });
        cx.run_until_parked();
        assert_eq!(told.get(), 1);
        assert_eq!(store.resampling_quality().unwrap(), ResamplingQuality::High);
        settings.read_with(cx, |settings, _| assert_eq!(settings.quality_ix(), 1));

        settings.update(cx, |settings, cx| settings.set_window("Hann", cx));
        cx.run_until_parked();
        assert_eq!(told.get(), 2);
        assert_eq!(
            store.resampling_quality().unwrap(),
            ResamplingQuality::Custom {
                fft_size: 1024,
                window: ResamplingWindow::Hann,
            }
        );
        settings.read_with(cx, |settings, _| {
            assert_eq!(settings.quality_ix(), 3);
            assert_eq!(settings.fft_size_value().as_ref(), "1024");
        });

        settings.update(cx, |settings, cx| settings.select_quality(2, cx));
        settings.read_with(cx, |settings, _| {
            assert_eq!(settings.fft_size_value().as_ref(), "4096");
            assert_eq!(settings.window_value().as_ref(), "Blackman-Harris²");
        });
    }

    #[gpui_kit::test]
    fn custom_values_are_told_saved_and_reach_the_output(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let (settings, told, _subscription) = settings_saving_to(&store, cx);
        settings.update(cx, |settings, cx| settings.select_quality(3, cx));

        settings.update(cx, |settings, cx| settings.set_fft_size("8192", cx));
        settings.update(cx, |settings, cx| settings.set_window("Hann²", cx));
        cx.run_until_parked();

        assert_eq!(told.get(), 2);
        assert_eq!(
            store.resampling_quality().unwrap(),
            ResamplingQuality::Custom {
                fft_size: 8192,
                window: ResamplingWindow::Hann2,
            }
        );
        settings.read_with(cx, |settings, _| {
            let resampling = settings.output().resampling();
            assert_eq!(resampling.fft_size(), 8192);
            assert_eq!(resampling.window(), audio::ResamplingWindow::Hann2);
            assert_eq!(settings.fft_size_value().as_ref(), "8192");
            assert_eq!(settings.window_value().as_ref(), "Hann²");
        });
    }

    #[gpui_kit::test]
    fn another_device_or_rate_keeps_the_resampling(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let (settings, _told, _subscription) = settings_saving_to(&store, cx);
        settings.update(cx, |settings, cx| settings.select_quality(1, cx));

        settings.update(cx, |settings, cx| settings.set_device("a:1", cx));
        settings.update(cx, |settings, cx| settings.set_rate("48000", cx));
        cx.run_until_parked();

        settings.read_with(cx, |settings, _| {
            assert_eq!(settings.output().resampling().fft_size(), 1024);
        });
    }

    fn labels(options: &[(SharedString, SharedString)]) -> Vec<&str> {
        options.iter().map(|(_, label)| label.as_ref()).collect()
    }

    fn values(options: &[(SharedString, SharedString)]) -> Vec<&str> {
        options.iter().map(|(value, _)| value.as_ref()).collect()
    }

    #[test]
    fn system_default_comes_first_and_devices_keep_their_ids() {
        let options = device_options(
            [("a:1", "Speakers"), ("a:2", "DAC")].into_iter(),
            None,
            true,
        );
        assert_eq!(labels(&options), ["System default", "Speakers", "DAC"]);
        assert_eq!(values(&options), [SYSTEM_DEFAULT, "a:1", "a:2"]);
    }

    #[test]
    fn a_chosen_device_that_is_gone_stays_an_option() {
        let options = device_options([("a:1", "Speakers")].into_iter(), Some("a:2"), true);
        assert_eq!(
            labels(&options),
            ["System default", "Speakers", "Disconnected device"]
        );
        assert_eq!(values(&options)[2], "a:2");
    }

    #[test]
    fn a_chosen_device_is_not_called_gone_before_the_first_listing() {
        let options = device_options(std::iter::empty(), Some("a:2"), false);
        assert_eq!(labels(&options), ["System default", "Loading"]);
    }

    #[test]
    fn rates_follow_the_device() {
        let options = rate_options(Some(&[44_100, 96_000]), None);
        assert_eq!(labels(&options), ["Match each song", "44.1 kHz", "96 kHz"]);
        assert_eq!(values(&options), [MATCH_SONG, "44100", "96000"]);
    }

    #[test]
    fn a_fixed_rate_is_not_called_unsupported_without_a_device() {
        let options = rate_options(None, Some(48_000));
        assert_eq!(labels(&options), ["Match each song", "48 kHz"]);
    }

    #[test]
    fn a_fixed_rate_the_device_lacks_stays_an_option() {
        let options = rate_options(Some(&[44_100]), Some(192_000));
        assert_eq!(
            labels(&options),
            ["Match each song", "44.1 kHz", "192 kHz (not supported)"]
        );
        assert_eq!(values(&options)[2], "192000");
    }
}
