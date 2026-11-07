use std::time::Duration;

use audio::{OutputDevice, OutputSettings, Transitions};
use config::{
    DsdOutput, R8brainParameters, RESAMPLING_FFT_SIZES, ResamplingQuality, ResamplingWindow,
    SAMPLE_RATE_MULTIPLES, SampleRate,
};
use gpui_kit::{AppContext as _, Context, EventEmitter, SharedString, Task};
use ui::format_sample_rate;

/// Device ids hold a colon, so none is equal to this.
pub(crate) const SYSTEM_DEFAULT: &str = "system-default";
pub(crate) const MATCH_SONG: &str = "match-song";
pub(crate) const RESAMPLERS: [&str; 2] = ["Rubato", "r8brain"];
pub(crate) const RUBATO_QUALITIES: [&str; 4] = ["Medium", "High", "Max", "Custom"];
const BASE_RATES: [u32; 2] = [44_100, 48_000];

pub struct OutputChanged;

pub struct TransitionsChanged;

pub struct AudioSettings {
    /// `None` without a home directory: choices then last until quitting.
    store: Option<config::Store>,
    output: OutputSettings,
    transitions: Transitions,
    quality: ResamplingQuality,
    /// Kept while r8brain is in use, to come back to.
    rubato: ResamplingQuality,
    /// Kept while Rubato is in use, to come back to.
    r8brain: R8brainParameters,
    /// What `r8brain` was when last saved, to write only what has changed since.
    saved_r8brain: R8brainParameters,
    dsd: DsdOutput,
    /// What `dsd` was when last saved, to write only what has changed since.
    saved_dsd: DsdOutput,
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
    exclusive: bool,
    trim_silence: bool,
    crossfade: bool,
    play_pause_fade: bool,
    quality: bool,
    r8brain: bool,
    dsd: bool,
}

impl EventEmitter<OutputChanged> for AudioSettings {}
impl EventEmitter<TransitionsChanged> for AudioSettings {}

impl AudioSettings {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let (store, output, quality, dsd, save_error) = read_saved_output();
        let (transitions, transitions_error) = read_saved_transitions(store.as_ref());
        let (r8brain, r8brain_error) = read_saved_r8brain(store.as_ref());
        let save_error = save_error.or(transitions_error).or(r8brain_error);
        let mut this = Self::with(store, output, quality, dsd, save_error);
        this.transitions = transitions;
        this.r8brain = r8brain;
        this.saved_r8brain = r8brain;
        this.list_devices(cx);
        this
    }

    fn with(
        store: Option<config::Store>,
        output: OutputSettings,
        quality: ResamplingQuality,
        dsd: DsdOutput,
        save_error: Option<SharedString>,
    ) -> Self {
        let r8brain = match quality {
            ResamplingQuality::R8brain(parameters) => parameters,
            _ => R8brainParameters::default(),
        };
        Self {
            store,
            output,
            transitions: Transitions::default(),
            quality,
            rubato: match quality {
                ResamplingQuality::R8brain(_) => ResamplingQuality::default(),
                quality => quality,
            },
            r8brain,
            saved_r8brain: r8brain,
            saved_dsd: dsd.clone(),
            dsd,
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

    pub fn transitions(&self) -> Transitions {
        self.transitions
    }

    pub(crate) fn is_trim_silence_on(&self) -> bool {
        self.transitions.trim_silence
    }

    pub(crate) fn set_trim_silence(&mut self, is_on: bool, cx: &mut Context<Self>) {
        if self.transitions.trim_silence != is_on {
            self.transitions.trim_silence = is_on;
            self.unsaved.trim_silence = true;
            self.transitions_changed(cx);
        }
    }

    pub(crate) fn is_play_pause_fade_on(&self) -> bool {
        self.transitions.fades_play_and_pause
    }

    pub(crate) fn set_play_pause_fade(&mut self, is_on: bool, cx: &mut Context<Self>) {
        if self.transitions.fades_play_and_pause != is_on {
            self.transitions.fades_play_and_pause = is_on;
            self.unsaved.play_pause_fade = true;
            self.transitions_changed(cx);
        }
    }

    pub(crate) fn crossfade_options() -> Vec<(SharedString, SharedString)> {
        std::iter::once(("0".into(), "Off".into()))
            .chain(
                (1..=config::LONGEST_CROSSFADE_SECONDS)
                    .map(|seconds| (seconds.to_string().into(), format!("{seconds} s").into())),
            )
            .collect()
    }

    pub(crate) fn crossfade_value(&self) -> SharedString {
        self.transitions.crossfade.as_secs().to_string().into()
    }

    pub(crate) fn set_crossfade(&mut self, value: &str, cx: &mut Context<Self>) {
        let crossfade = Duration::from_secs(value.parse().unwrap_or(0));
        if self.transitions.crossfade != crossfade {
            self.transitions.crossfade = crossfade;
            self.unsaved.crossfade = true;
            self.transitions_changed(cx);
        }
    }

    /// Why some songs will not crossfade, when the device has to switch its
    /// rate between them.
    pub(crate) fn crossfade_note(&self) -> Option<&'static str> {
        (!self.transitions.crossfade.is_zero()
            && !matches!(self.output.sample_rate(), SampleRate::Fixed(_)))
        .then_some(
            "Songs at another sample rate play apart. Set a fixed Sample rate to crossfade them too.",
        )
    }

    fn transitions_changed(&mut self, cx: &mut Context<Self>) {
        self.save(cx);
        cx.emit(TransitionsChanged);
        cx.notify();
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
        let output = self.output_on(device, self.output.sample_rate());
        if self.output != output {
            self.unsaved.device = true;
            self.set_output(output, cx);
        }
    }

    pub(crate) fn rate_options(&self) -> Vec<(SharedString, SharedString)> {
        rate_options(
            self.selected_device_rates(),
            self.output.sample_rate(),
            self.output.can_run_at_song_multiple(),
        )
    }

    pub(crate) fn rate_value(&self) -> SharedString {
        match self.output.sample_rate() {
            SampleRate::MatchSong => MATCH_SONG.into(),
            rate => rate.text().into(),
        }
    }

    pub(crate) fn set_rate(&mut self, value: &str, cx: &mut Context<Self>) {
        let device = self.output.device().map(str::to_owned);
        let output = self.output_on(device, SampleRate::parse(value).unwrap_or_default());
        if self.output != output {
            self.unsaved.rate = true;
            self.set_output(output, cx);
        }
    }

    pub(crate) fn is_exclusive_offered() -> bool {
        audio::CAN_PLAY_EXCLUSIVE
    }

    /// DSD holds the device whatever the setting.
    pub(crate) fn is_exclusive_on(&self) -> bool {
        self.output.is_exclusive() || self.output.dop().is_some()
    }

    pub(crate) fn set_exclusive(&mut self, is_on: bool, cx: &mut Context<Self>) {
        let output = self.output.clone().with_exclusive(is_on);
        if self.output != output {
            self.unsaved.exclusive = true;
            self.set_output(output, cx);
        }
    }

    pub(crate) fn resampler_ix(&self) -> usize {
        usize::from(self.uses_r8brain())
    }

    /// `ix` is into [`RESAMPLERS`]. Each comes back at the quality it had.
    pub(crate) fn select_resampler(&mut self, ix: usize, cx: &mut Context<Self>) {
        let quality = match ix {
            0 => self.rubato,
            _ => ResamplingQuality::R8brain(self.r8brain),
        };
        self.set_quality(quality, cx);
    }

    pub(crate) fn rubato_quality_ix(&self) -> usize {
        match self.rubato {
            ResamplingQuality::Medium => 0,
            ResamplingQuality::High => 1,
            ResamplingQuality::Max => 2,
            ResamplingQuality::Custom { .. } | ResamplingQuality::R8brain(_) => 3,
        }
    }

    pub(crate) fn uses_r8brain(&self) -> bool {
        matches!(self.quality, ResamplingQuality::R8brain(_))
    }

    pub(crate) fn r8brain_parameters(&self) -> R8brainParameters {
        self.r8brain
    }

    /// A field's change can settle after Rubato was chosen, and Rubato stays.
    pub(crate) fn set_r8brain_parameters(
        &mut self,
        parameters: R8brainParameters,
        cx: &mut Context<Self>,
    ) {
        if self.r8brain == parameters {
            return;
        }
        self.r8brain = parameters;
        self.unsaved.r8brain = true;
        if self.uses_r8brain() {
            self.set_quality(ResamplingQuality::R8brain(parameters), cx);
        } else {
            self.save(cx);
            cx.notify();
        }
    }

    /// `ix` is into [`RUBATO_QUALITIES`], and Rubato resamples at it. Custom starts from the
    /// values Rubato had, so choosing it changes nothing until one of them is changed.
    pub(crate) fn select_rubato_quality(&mut self, ix: usize, cx: &mut Context<Self>) {
        let quality = match ix {
            0 => ResamplingQuality::Medium,
            1 => ResamplingQuality::High,
            2 => ResamplingQuality::Max,
            _ => ResamplingQuality::Custom {
                fft_size: self.rubato.fft_size(),
                window: self.rubato.window(),
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
        self.rubato.fft_size().to_string().into()
    }

    /// Another size than the one in use makes the quality custom.
    pub(crate) fn set_fft_size(&mut self, value: &str, cx: &mut Context<Self>) {
        let Ok(fft_size) = value.parse() else {
            return;
        };
        if fft_size != self.rubato.fft_size() {
            let window = self.rubato.window();
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
        window_label(self.rubato.window()).into()
    }

    /// Another window than the one in use makes the quality custom.
    pub(crate) fn set_window(&mut self, value: &str, cx: &mut Context<Self>) {
        let window = ResamplingWindow::ALL
            .into_iter()
            .find(|window| window_label(*window) == value);
        if let Some(window) = window.filter(|window| *window != self.rubato.window()) {
            let fft_size = self.rubato.fft_size();
            self.set_quality(ResamplingQuality::Custom { fft_size, window }, cx);
        }
    }

    fn set_quality(&mut self, quality: ResamplingQuality, cx: &mut Context<Self>) {
        if self.quality == quality {
            return;
        }
        self.quality = quality;
        if !matches!(quality, ResamplingQuality::R8brain(_)) {
            self.rubato = quality;
        }
        self.unsaved.quality = true;
        let device = self.output.device().map(str::to_owned);
        let output = self.output_on(device, self.output.sample_rate());
        if self.output == output {
            self.save(cx);
            cx.notify();
        } else {
            self.set_output(output, cx);
        }
    }

    fn selected_device_rates(&self) -> Option<&[u32]> {
        self.selected_device().map(|device| {
            if self.output.is_exclusive() {
                device.exclusive_sample_rates()
            } else {
                device.sample_rates()
            }
        })
    }

    pub(crate) fn selected_device(&self) -> Option<&OutputDevice> {
        let mut devices = self.devices.iter().flatten();
        match self.output.device() {
            Some(id) => devices.find(|device| device.id() == Some(id)),
            None => devices.find(|device| device.is_default()),
        }
    }

    fn output_on(&self, device: Option<String>, rate: SampleRate) -> OutputSettings {
        OutputSettings::from_config(device, rate, self.quality, &self.dsd)
            .with_exclusive(self.output.is_exclusive())
    }

    pub(crate) fn dsd(&self) -> &DsdOutput {
        &self.dsd
    }

    pub(crate) fn set_dsd(&mut self, dsd: DsdOutput, cx: &mut Context<Self>) {
        if self.dsd == dsd {
            return;
        }
        self.dsd = dsd;
        self.unsaved.dsd = true;
        let device = self.output.device().map(str::to_owned);
        let output = self.output_on(device, self.output.sample_rate());
        if self.output == output {
            self.save(cx);
            cx.notify();
        } else {
            self.set_output(output, cx);
        }
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
            while let Ok(Some((
                output,
                transitions,
                quality,
                saved_r8brain,
                r8brain,
                saved_dsd,
                dsd,
                unsaved,
            ))) = this.update(cx, |this, _| {
                let unsaved = std::mem::take(&mut this.unsaved);
                this.saving = unsaved.device
                    || unsaved.rate
                    || unsaved.exclusive
                    || unsaved.trim_silence
                    || unsaved.crossfade
                    || unsaved.play_pause_fade
                    || unsaved.quality
                    || unsaved.r8brain
                    || unsaved.dsd;
                let saved_r8brain = std::mem::replace(&mut this.saved_r8brain, this.r8brain);
                let saved_dsd = std::mem::replace(&mut this.saved_dsd, this.dsd.clone());
                this.saving.then(|| {
                    let dsd = this.dsd.clone();
                    (
                        this.output.clone(),
                        this.transitions,
                        this.quality,
                        saved_r8brain,
                        this.r8brain,
                        saved_dsd,
                        dsd,
                        unsaved,
                    )
                })
            }) {
                let store = store.clone();
                let saved = cx
                    .background_spawn(async move {
                        if unsaved.device {
                            store.save_output_device(output.device())?;
                        }
                        if unsaved.rate {
                            store.save_sample_rate(output.sample_rate())?;
                        }
                        if unsaved.exclusive {
                            store.save_exclusive_mode(output.is_exclusive())?;
                        }
                        if unsaved.trim_silence {
                            store.save_trim_silence(transitions.trim_silence)?;
                        }
                        if unsaved.crossfade {
                            store.save_crossfade_seconds(
                                u32::try_from(transitions.crossfade.as_secs()).unwrap_or(0),
                            )?;
                        }
                        if unsaved.play_pause_fade {
                            store.save_play_pause_fade(transitions.fades_play_and_pause)?;
                        }
                        if unsaved.r8brain {
                            store.save_r8brain_changes(saved_r8brain, r8brain)?;
                        }
                        if unsaved.quality {
                            store.save_resampling_quality(quality)?;
                        }
                        if unsaved.dsd {
                            store.save_dsd_changes(&saved_dsd, &dsd)?;
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
    DsdOutput,
    Option<SharedString>,
) {
    let unread = |store, error| {
        let quality = ResamplingQuality::default();
        let dsd = DsdOutput::default();
        let output = OutputSettings::from_config(None, SampleRate::MatchSong, quality, &dsd);
        (store, output, quality, dsd, Some(error))
    };
    let store = match config::Store::user() {
        Ok(store) => store,
        Err(error) => {
            return unread(
                None,
                format!("Audio settings won't be saved: {error}.").into(),
            );
        }
    };
    match (
        store.output_device(),
        store.sample_rate(),
        store.exclusive_mode(),
        store.resampling_quality(),
        store.dsd_output(),
    ) {
        (Ok(device), Ok(rate), Ok(exclusive), Ok(quality), Ok(dsd)) => {
            let output =
                OutputSettings::from_config(device, rate, quality, &dsd).with_exclusive(exclusive);
            (Some(store), output, quality, dsd, None)
        }
        (Err(error), _, _, _, _)
        | (_, Err(error), _, _, _)
        | (_, _, Err(error), _, _)
        | (_, _, _, Err(error), _)
        | (_, _, _, _, Err(error)) => {
            let error = format!("Couldn't read the audio settings: {error}.").into();
            unread(Some(store), error)
        }
    }
}

/// What the file says about how the sound starts, stops and moves between
/// songs, and why that could not be read. No store means the output's own
/// error has said so already.
fn read_saved_transitions(store: Option<&config::Store>) -> (Transitions, Option<SharedString>) {
    let Some(store) = store else {
        return (Transitions::default(), None);
    };
    match (
        store.trim_silence(),
        store.crossfade_seconds(),
        store.play_pause_fade(),
    ) {
        (Ok(trim_silence), Ok(seconds), Ok(fades_play_and_pause)) => (
            Transitions {
                trim_silence,
                crossfade: Duration::from_secs(u64::from(seconds)),
                fades_play_and_pause,
            },
            None,
        ),
        (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => (
            Transitions::default(),
            Some(format!("Couldn't read the audio settings: {error}.").into()),
        ),
    }
}

/// What the file says about the r8brain resampling, whatever is in use, and why that could not
/// be read. No store means the output's own error has said so already.
fn read_saved_r8brain(store: Option<&config::Store>) -> (R8brainParameters, Option<SharedString>) {
    let Some(store) = store else {
        return (R8brainParameters::default(), None);
    };
    match store.r8brain_parameters() {
        Ok(parameters) => (parameters, None),
        Err(error) => (
            R8brainParameters::default(),
            Some(format!("Couldn't read the audio settings: {error}.").into()),
        ),
    }
}

pub(crate) fn window_label(window: ResamplingWindow) -> &'static str {
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

/// `rates` are what the device supports, if it is listed. A multiple is offered
/// if it fits either family of rates. A chosen rate or multiple that does not
/// fit stays an option; the engine plays at what the device can do meanwhile.
/// Without a listed device, nothing is known to be unsupported.
fn rate_options(
    rates: Option<&[u32]>,
    selected: SampleRate,
    can_run_at_multiple: bool,
) -> Vec<(SharedString, SharedString)> {
    let listed = rates.unwrap_or_default();
    let fits = |multiple: u32| {
        can_run_at_multiple
            && BASE_RATES
                .iter()
                .any(|base| listed.contains(&(base * multiple)))
    };
    let multiple_text = |multiple: u32| SampleRate::SongMultiple(multiple).text();
    let rate_option = |rate: u32, label: String| (rate.to_string().into(), label.into());
    let unsupported = |label: String| match rates {
        Some(_) => format!("{label} (not supported)"),
        None => label,
    };

    let mut options: Vec<(SharedString, SharedString)> =
        vec![(MATCH_SONG.into(), "Match each song".into())];
    options.extend(
        SAMPLE_RATE_MULTIPLES
            .into_iter()
            .filter(|multiple| fits(*multiple))
            .map(|multiple| {
                let text = SharedString::from(multiple_text(multiple));
                (text.clone(), text)
            }),
    );
    options.extend(
        listed
            .iter()
            .map(|rate| rate_option(*rate, format_sample_rate(*rate))),
    );
    match selected {
        SampleRate::Fixed(rate) if !listed.contains(&rate) => {
            options.push(rate_option(rate, unsupported(format_sample_rate(rate))));
        }
        SampleRate::SongMultiple(multiple) if !fits(multiple) => {
            let text = multiple_text(multiple);
            options.push((text.clone().into(), unsupported(text).into()));
        }
        _ => {}
    }
    options
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use gpui_kit::{Entity, TestAppContext};

    use super::*;

    /// `AudioSettings::new` would list the machine's devices.
    pub(crate) fn settings_saving_to(
        store: &config::Store,
        cx: &mut TestAppContext,
    ) -> (Entity<AudioSettings>, Rc<Cell<u32>>, gpui_kit::Subscription) {
        let quality = ResamplingQuality::default();
        let settings = cx.new(|_| {
            let dsd = DsdOutput::default();
            AudioSettings::with(
                Some(store.clone()),
                OutputSettings::from_config(None, SampleRate::MatchSong, quality, &dsd),
                quality,
                dsd,
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

        settings.update(cx, |settings, cx| settings.select_rubato_quality(2, cx));
        cx.run_until_parked();

        assert_eq!(told.get(), 1);
        assert_eq!(store.resampling_quality().unwrap(), ResamplingQuality::Max);
        settings.read_with(cx, |settings, _| {
            assert_eq!(settings.rubato_quality_ix(), 2);
            assert_eq!(
                settings.output().resampling(),
                audio::Resampling::new(4096, audio::ResamplingWindow::BlackmanHarris2)
            );
        });
    }

    #[gpui_kit::test]
    fn r8brain_is_told_saved_and_reaches_the_output(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let (settings, told, _subscription) = settings_saving_to(&store, cx);

        settings.update(cx, |settings, cx| settings.select_resampler(1, cx));
        cx.run_until_parked();

        assert_eq!(told.get(), 1);
        assert_eq!(
            store.resampling_quality().unwrap(),
            ResamplingQuality::R8brain(R8brainParameters::default())
        );
        settings.read_with(cx, |settings, _| {
            assert_eq!(settings.resampler_ix(), 1);
            assert!(settings.uses_r8brain());
            assert_eq!(
                settings.output().resampling(),
                audio::Resampling::R8brain(audio::R8brainSettings::default())
            );
        });
    }

    #[gpui_kit::test]
    fn r8brain_values_from_elsewhere_stay_out_of_the_file(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let mut filter = config::R8brainQuality::High.filter();
        filter.attenuation = 100.0;
        let mut from_environment = R8brainParameters::default();
        from_environment.quality = config::R8brainQuality::Custom(filter);
        let quality = ResamplingQuality::R8brain(from_environment);
        let settings = cx.new(|_| {
            let dsd = DsdOutput::default();
            let output = OutputSettings::from_config(None, SampleRate::MatchSong, quality, &dsd);
            AudioSettings::with(Some(store.clone()), output, quality, dsd, None)
        });

        settings.update(cx, |settings, cx| {
            settings.change_r8brain(
                |parameters| parameters.performance.max_input_frames = 4096,
                cx,
            );
        });
        cx.run_until_parked();

        let text = std::fs::read_to_string(store.config_path()).unwrap();
        assert!(text.contains("r8brain_max_input_frames = 4096"), "{text}");
        assert!(!text.contains("r8brain_attenuation"), "{text}");
    }

    #[gpui_kit::test]
    fn each_resampler_comes_back_at_the_quality_it_had(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let (settings, _told, _subscription) = settings_saving_to(&store, cx);
        settings.update(cx, |settings, cx| {
            settings.select_rubato_quality(1, cx);
            settings.select_resampler(1, cx);
            settings.select_r8brain_quality(0, cx);
            settings.select_resampler(0, cx);
        });
        cx.run_until_parked();
        assert_eq!(store.resampling_quality().unwrap(), ResamplingQuality::High);
        settings.read_with(cx, |settings, _| {
            assert_eq!(settings.resampler_ix(), 0);
            assert_eq!(settings.rubato_quality_ix(), 1);
        });

        settings.update(cx, |settings, cx| settings.select_resampler(1, cx));
        settings.read_with(cx, |settings, _| {
            assert_eq!(settings.r8brain_quality_ix(), 0);
        });
    }

    #[gpui_kit::test]
    fn custom_after_r8brain_starts_from_the_medium_values(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let (settings, _told, _subscription) = settings_saving_to(&store, cx);
        settings.update(cx, |settings, cx| settings.select_resampler(1, cx));

        settings.update(cx, |settings, cx| settings.select_rubato_quality(3, cx));
        cx.run_until_parked();

        assert_eq!(
            store.resampling_quality().unwrap(),
            ResamplingQuality::Custom {
                fft_size: 256,
                window: ResamplingWindow::BlackmanHarris2,
            }
        );
        settings.read_with(cx, |settings, _| {
            assert!(!settings.uses_r8brain());
            assert_eq!(
                settings.output().resampling(),
                audio::Resampling::new(256, audio::ResamplingWindow::BlackmanHarris2)
            );
        });
    }

    #[gpui_kit::test]
    fn choosing_custom_is_saved_but_keeps_the_sound(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let (settings, told, _subscription) = settings_saving_to(&store, cx);
        settings.update(cx, |settings, cx| settings.select_rubato_quality(1, cx));
        cx.run_until_parked();
        assert_eq!(told.get(), 1);

        settings.update(cx, |settings, cx| settings.select_rubato_quality(3, cx));
        cx.run_until_parked();

        assert_eq!(told.get(), 1);
        let custom = ResamplingQuality::Custom {
            fft_size: 1024,
            window: ResamplingWindow::BlackmanHarris2,
        };
        assert_eq!(store.resampling_quality().unwrap(), custom);
        settings.read_with(cx, |settings, _| {
            assert_eq!(settings.rubato_quality_ix(), 3)
        });
    }

    #[gpui_kit::test]
    fn another_value_turns_a_preset_custom_and_the_same_one_does_not(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let (settings, told, _subscription) = settings_saving_to(&store, cx);
        settings.update(cx, |settings, cx| settings.select_rubato_quality(1, cx));
        cx.run_until_parked();
        assert_eq!(told.get(), 1);

        settings.update(cx, |settings, cx| {
            settings.set_fft_size("1024", cx);
            settings.set_window("Blackman-Harris²", cx);
        });
        cx.run_until_parked();
        assert_eq!(told.get(), 1);
        assert_eq!(store.resampling_quality().unwrap(), ResamplingQuality::High);
        settings.read_with(cx, |settings, _| {
            assert_eq!(settings.rubato_quality_ix(), 1)
        });

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
            assert_eq!(settings.rubato_quality_ix(), 3);
            assert_eq!(settings.fft_size_value().as_ref(), "1024");
        });

        settings.update(cx, |settings, cx| settings.select_rubato_quality(2, cx));
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
        settings.update(cx, |settings, cx| settings.select_rubato_quality(3, cx));

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
            assert_eq!(
                settings.output().resampling(),
                audio::Resampling::new(8192, audio::ResamplingWindow::Hann2)
            );
            assert_eq!(settings.fft_size_value().as_ref(), "8192");
            assert_eq!(settings.window_value().as_ref(), "Hann²");
        });
    }

    #[gpui_kit::test]
    fn another_device_or_rate_keeps_the_resampling(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let (settings, _told, _subscription) = settings_saving_to(&store, cx);
        settings.update(cx, |settings, cx| settings.select_rubato_quality(1, cx));

        settings.update(cx, |settings, cx| settings.set_device("a:1", cx));
        settings.update(cx, |settings, cx| settings.set_rate("48000", cx));
        cx.run_until_parked();

        settings.read_with(cx, |settings, _| {
            assert_eq!(
                settings.output().resampling(),
                audio::Resampling::new(1024, audio::ResamplingWindow::BlackmanHarris2)
            );
        });
    }

    #[gpui_kit::test]
    fn transitions_are_told_saved_and_leave_the_output_alone(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let (settings, told_output, _subscription) = settings_saving_to(&store, cx);
        let told = Rc::new(Cell::new(0));
        let _told = cx.update(|cx| {
            let told = told.clone();
            cx.subscribe(&settings, move |_, _: &TransitionsChanged, _| {
                told.set(told.get() + 1);
            })
        });

        settings.update(cx, |settings, cx| {
            settings.set_trim_silence(true, cx);
            settings.set_trim_silence(true, cx);
            settings.set_crossfade("5", cx);
            settings.set_crossfade("5", cx);
            settings.set_play_pause_fade(true, cx);
            settings.set_play_pause_fade(true, cx);
        });
        cx.run_until_parked();
        assert_eq!(told.get(), 3);
        assert_eq!(told_output.get(), 0);
        assert!(store.trim_silence().unwrap());
        assert_eq!(store.crossfade_seconds().unwrap(), 5);
        assert!(store.play_pause_fade().unwrap());
        settings.read_with(cx, |settings, _| {
            assert_eq!(settings.crossfade_value().as_ref(), "5");
            assert!(settings.is_trim_silence_on());
            assert_eq!(settings.transitions().crossfade, Duration::from_secs(5));
        });

        settings.update(cx, |settings, cx| {
            settings.set_trim_silence(false, cx);
            settings.set_crossfade("0", cx);
            settings.set_play_pause_fade(false, cx);
        });
        cx.run_until_parked();
        assert!(!store.trim_silence().unwrap());
        assert_eq!(store.crossfade_seconds().unwrap(), 0);
        assert!(!store.play_pause_fade().unwrap());
    }

    #[test]
    fn saved_transitions_come_back_and_bad_ones_are_told() {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        store.save_trim_silence(true).unwrap();
        store.save_crossfade_seconds(3).unwrap();
        store.save_play_pause_fade(true).unwrap();
        let (transitions, error) = read_saved_transitions(Some(&store));
        assert!(transitions.trim_silence);
        assert_eq!(transitions.crossfade, Duration::from_secs(3));
        assert!(transitions.fades_play_and_pause);
        assert!(error.is_none());

        store
            .save_crossfade_seconds(config::LONGEST_CROSSFADE_SECONDS + 1)
            .unwrap();
        let (transitions, error) = read_saved_transitions(Some(&store));
        assert_eq!(transitions, Transitions::default());
        assert!(error.is_some());
    }

    #[gpui_kit::test]
    fn the_crossfade_note_says_when_the_device_may_have_to_switch(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let (settings, _told, _subscription) = settings_saving_to(&store, cx);
        settings.read_with(cx, |settings, _| {
            assert!(settings.crossfade_note().is_none());
        });
        settings.update(cx, |settings, cx| settings.set_crossfade("5", cx));
        settings.read_with(cx, |settings, _| {
            assert!(settings.crossfade_note().is_some());
        });
        settings.update(cx, |settings, cx| settings.set_rate("48000", cx));
        settings.read_with(cx, |settings, _| {
            assert!(settings.crossfade_note().is_none());
        });
    }

    #[gpui_kit::test]
    fn exclusive_mode_is_told_saved_and_survives_other_changes(cx: &mut TestAppContext) {
        if !audio::CAN_PLAY_EXCLUSIVE {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let (settings, told, _subscription) = settings_saving_to(&store, cx);

        settings.update(cx, |settings, cx| settings.set_exclusive(true, cx));
        settings.update(cx, |settings, cx| settings.set_exclusive(true, cx));
        cx.run_until_parked();
        assert_eq!(told.get(), 1);
        assert!(store.exclusive_mode().unwrap());

        settings.update(cx, |settings, cx| {
            settings.set_device("a:1", cx);
            settings.set_rate("48000", cx);
            settings.select_rubato_quality(2, cx);
        });
        cx.run_until_parked();
        settings.read_with(cx, |settings, _| {
            assert!(settings.output().is_exclusive());
        });

        settings.update(cx, |settings, cx| {
            let mut dsd = settings.dsd().clone();
            dsd.is_on = true;
            dsd.device = Some("a:1".to_owned());
            settings.set_dsd(dsd, cx);
        });
        cx.run_until_parked();
        settings.read_with(cx, |settings, _| {
            assert!(settings.output().is_exclusive());
        });
        settings.update(cx, |settings, cx| {
            let mut dsd = settings.dsd().clone();
            dsd.is_on = false;
            settings.set_dsd(dsd, cx);
        });

        settings.update(cx, |settings, cx| settings.set_exclusive(false, cx));
        cx.run_until_parked();
        assert!(!store.exclusive_mode().unwrap());
        settings.read_with(cx, |settings, _| assert!(!settings.is_exclusive_on()));
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
        let options = rate_options(Some(&[44_100, 96_000]), SampleRate::MatchSong, true);
        assert_eq!(
            labels(&options),
            ["Match each song", "x2", "44.1 kHz", "96 kHz"]
        );
        assert_eq!(values(&options), [MATCH_SONG, "x2", "44100", "96000"]);
    }

    #[test]
    fn a_fixed_rate_is_not_called_unsupported_without_a_device() {
        let options = rate_options(None, SampleRate::Fixed(48_000), true);
        assert_eq!(labels(&options), ["Match each song", "48 kHz"]);
    }

    #[test]
    fn a_fixed_rate_the_device_lacks_stays_an_option() {
        let options = rate_options(Some(&[44_100]), SampleRate::Fixed(192_000), true);
        assert_eq!(
            labels(&options),
            ["Match each song", "44.1 kHz", "192 kHz (not supported)"]
        );
        assert_eq!(values(&options)[2], "192000");
    }

    #[test]
    fn a_multiple_is_offered_when_either_family_reaches_it() {
        let options = rate_options(
            Some(&[48_000, 176_400, 192_000]),
            SampleRate::MatchSong,
            true,
        );
        assert_eq!(
            values(&options),
            [MATCH_SONG, "x4", "48000", "176400", "192000"]
        );
    }

    #[test]
    fn a_multiple_the_device_lacks_stays_an_option() {
        let options = rate_options(Some(&[44_100]), SampleRate::SongMultiple(4), true);
        assert_eq!(
            labels(&options),
            ["Match each song", "44.1 kHz", "x4 (not supported)"]
        );
        assert_eq!(values(&options)[2], "x4");
    }

    #[test]
    fn a_multiple_is_not_offered_where_it_cannot_reach_the_device() {
        let rates: &[u32] = &[44_100, 96_000];
        let options = rate_options(Some(rates), SampleRate::MatchSong, false);
        assert_eq!(values(&options), [MATCH_SONG, "44100", "96000"]);

        let options = rate_options(Some(rates), SampleRate::SongMultiple(2), false);
        assert_eq!(
            labels(&options),
            [
                "Match each song",
                "44.1 kHz",
                "96 kHz",
                "x2 (not supported)"
            ]
        );
    }

    #[gpui_kit::test]
    fn a_multiple_is_told_saved_and_reaches_the_output(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let (settings, told, _subscription) = settings_saving_to(&store, cx);

        settings.update(cx, |settings, cx| settings.set_rate("x4", cx));
        cx.run_until_parked();

        assert_eq!(told.get(), 1);
        assert_eq!(store.sample_rate().unwrap(), SampleRate::SongMultiple(4));
        settings.read_with(cx, |settings, _| {
            assert_eq!(settings.output().sample_rate(), SampleRate::SongMultiple(4));
            assert_eq!(settings.rate_value().as_ref(), "x4");
        });

        settings.update(cx, |settings, cx| settings.set_rate(MATCH_SONG, cx));
        cx.run_until_parked();
        assert_eq!(store.sample_rate().unwrap(), SampleRate::MatchSong);
    }
}
