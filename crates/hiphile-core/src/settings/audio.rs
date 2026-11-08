mod choices;
mod dsd;
mod r8brain;
mod soxr;

use std::mem;
use std::sync::{Arc, Weak};
use std::time::Duration;

use audio::{OutputDevice, OutputSettings, Transitions};
use config::{
    DsdOutput, R8brainParameters, RESAMPLING_FFT_SIZES, ResamplingQuality, ResamplingWindow,
    SAMPLE_RATE_MULTIPLES, SampleRate, SoxrParameters,
};
use executor::{Executor, Saver, Task};
use playback::format_sample_rate;
use tokio::sync::watch;

use executor::{Publish, Published};

pub use choices::Choice;
pub use r8brain::{R8BRAIN_PHASES, R8BRAIN_QUALITIES};
pub use soxr::{SOXR_INTERPOLATIONS, SOXR_PHASES, SOXR_QUALITIES};

/// Device ids hold a colon, so none is equal to this.
pub const SYSTEM_DEFAULT: &str = "system-default";
pub const MATCH_SONG: &str = "match-song";
pub const RESAMPLERS: [&str; 3] = ["Rubato", "r8brain", "soxr"];
const R8BRAIN_IX: usize = 1;
const SOXR_IX: usize = 2;
pub const RUBATO_QUALITIES: [&str; 4] = ["Medium", "High", "Max", "Custom"];
const BASE_RATES: [u32; 2] = [44_100, 48_000];

/// The output and how the sound moves between songs, with what is needed to choose them.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioSnapshot {
    output: OutputSettings,
    transitions: Transitions,
    quality: ResamplingQuality,
    /// Kept while another resampler is in use, to come back to.
    rubato: ResamplingQuality,
    /// Kept while another resampler is in use, to come back to.
    r8brain: R8brainParameters,
    /// Kept while another resampler is in use, to come back to.
    soxr: SoxrParameters,
    dsd: DsdOutput,
    /// `None` until first listed.
    devices: Option<Arc<[OutputDevice]>>,
    list_error: Option<String>,
    save_error: Option<String>,
}

impl AudioSnapshot {
    pub fn output(&self) -> &OutputSettings {
        &self.output
    }

    pub fn transitions(&self) -> Transitions {
        self.transitions
    }

    pub fn is_trim_silence_on(&self) -> bool {
        self.transitions.trim_silence
    }

    pub fn is_play_pause_fade_on(&self) -> bool {
        self.transitions.fades_play_and_pause
    }

    pub fn crossfade_value(&self) -> String {
        self.transitions.crossfade.as_secs().to_string()
    }

    /// Why some songs will not crossfade, when the device has to switch its rate between them.
    pub fn crossfade_note(&self) -> Option<&'static str> {
        (!self.transitions.crossfade.is_zero()
            && !matches!(self.output.sample_rate(), SampleRate::Fixed(_)))
        .then_some(
            "Songs at another sample rate play apart. Set a fixed Sample rate to crossfade them too.",
        )
    }

    pub fn has_devices(&self) -> bool {
        self.devices.is_some()
    }

    pub fn errors(&self) -> impl Iterator<Item = &str> {
        self.list_error
            .iter()
            .chain(&self.save_error)
            .map(String::as_str)
    }

    pub fn device_options(&self) -> Vec<Choice> {
        let devices = self
            .devices
            .iter()
            .flat_map(|devices| devices.iter())
            .filter_map(|device| device.id().map(|id| (id, device.name())));
        device_options(devices, self.output.device(), self.has_devices())
    }

    pub fn device_value(&self) -> String {
        self.output
            .device()
            .map_or(SYSTEM_DEFAULT.to_owned(), str::to_owned)
    }

    pub fn rate_options(&self) -> Vec<Choice> {
        rate_options(
            self.selected_device_rates(),
            self.output.sample_rate(),
            self.output.can_run_at_song_multiple(),
        )
    }

    pub fn rate_value(&self) -> String {
        match self.output.sample_rate() {
            SampleRate::MatchSong => MATCH_SONG.to_owned(),
            rate => rate.text(),
        }
    }

    /// DSD holds the device whatever the setting.
    pub fn is_exclusive_on(&self) -> bool {
        self.output.is_exclusive() || self.output.dop().is_some()
    }

    /// Into [`RESAMPLERS`].
    pub fn resampler_ix(&self) -> usize {
        match self.quality {
            ResamplingQuality::R8brain(_) => R8BRAIN_IX,
            ResamplingQuality::Soxr(_) => SOXR_IX,
            _ => 0,
        }
    }

    /// Into [`RUBATO_QUALITIES`]. Rubato keeps none of the other resamplers' qualities.
    pub fn rubato_quality_ix(&self) -> usize {
        match self.rubato {
            ResamplingQuality::Medium => 0,
            ResamplingQuality::High => 1,
            ResamplingQuality::Max => 2,
            ResamplingQuality::Custom { .. }
            | ResamplingQuality::R8brain(_)
            | ResamplingQuality::Soxr(_) => 3,
        }
    }

    pub fn uses_r8brain(&self) -> bool {
        matches!(self.quality, ResamplingQuality::R8brain(_))
    }

    pub fn uses_soxr(&self) -> bool {
        matches!(self.quality, ResamplingQuality::Soxr(_))
    }

    pub fn soxr_parameters(&self) -> SoxrParameters {
        self.soxr
    }

    pub fn r8brain_parameters(&self) -> R8brainParameters {
        self.r8brain
    }

    pub fn fft_size_options(&self) -> Vec<Choice> {
        choices::same_labels(RESAMPLING_FFT_SIZES.iter().map(u32::to_string))
    }

    pub fn fft_size_value(&self) -> String {
        self.rubato.fft_size().to_string()
    }

    pub fn window_options(&self) -> Vec<Choice> {
        ResamplingWindow::ALL
            .iter()
            .map(|window| Choice::new(window_label(*window), window_label(*window)))
            .collect()
    }

    pub fn window_value(&self) -> String {
        window_label(self.rubato.window()).to_owned()
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

    pub fn selected_device(&self) -> Option<&OutputDevice> {
        let mut devices = self.devices.iter().flat_map(|devices| devices.iter());
        match self.output.device() {
            Some(id) => devices.find(|device| device.id() == Some(id)),
            None => devices.find(|device| device.is_default()),
        }
    }

    pub fn dsd(&self) -> &DsdOutput {
        &self.dsd
    }
}

/// The output device, the sample rate, the resampling, DSD and how the sound moves between songs.
/// Each change is saved, and only what was changed is written: the rest may have come from the
/// environment, or from a file that could not be read.
#[derive(Clone)]
pub struct AudioSettings(Arc<Inner>);

struct Inner {
    executor: Executor,
    /// `None` without a home directory: choices then last until quitting.
    saver: Option<Saver<Changes>>,
    state: Published<State>,
}

struct State {
    settings: AudioSnapshot,
    /// What `r8brain` comes to on disk once the saver has written what it was given, to write
    /// only what has changed since.
    written_r8brain: R8brainParameters,
    /// As `written_r8brain`.
    written_soxr: SoxrParameters,
    /// As `written_r8brain`.
    written_dsd: DsdOutput,
    /// What the change under way has changed.
    unsaved: Unsaved,
    is_listing: bool,
    _list: Task<()>,
    #[cfg(test)]
    told: tests::Told,
}

impl Publish for State {
    type Snapshot = AudioSnapshot;

    fn snapshot(&self) -> &AudioSnapshot {
        &self.settings
    }
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
    soxr: bool,
    dsd: bool,
}

impl Unsaved {
    fn is_any(self) -> bool {
        self.device
            || self.rate
            || self.exclusive
            || self.trim_silence
            || self.crossfade
            || self.play_pause_fade
            || self.quality
            || self.r8brain
            || self.soxr
            || self.dsd
    }

    fn and(self, other: Self) -> Self {
        Self {
            device: self.device || other.device,
            rate: self.rate || other.rate,
            exclusive: self.exclusive || other.exclusive,
            trim_silence: self.trim_silence || other.trim_silence,
            crossfade: self.crossfade || other.crossfade,
            play_pause_fade: self.play_pause_fade || other.play_pause_fade,
            quality: self.quality || other.quality,
            r8brain: self.r8brain || other.r8brain,
            soxr: self.soxr || other.soxr,
            dsd: self.dsd || other.dsd,
        }
    }
}

/// What a write is to do: the values that `unsaved` names, and for the parameters, what they
/// were on disk before, so that only those that differ are written.
struct Changes {
    unsaved: Unsaved,
    output: OutputSettings,
    transitions: Transitions,
    quality: ResamplingQuality,
    r8brain: (R8brainParameters, R8brainParameters),
    soxr: (SoxrParameters, SoxrParameters),
    dsd: (DsdOutput, DsdOutput),
}

impl AudioSettings {
    /// Lists no devices until asked to.
    pub fn new(executor: Executor, store: Result<config::Store, config::Error>) -> Self {
        let (store, output, quality, dsd, save_error) = read_saved_output(store);
        let (transitions, transitions_error) = read_saved_transitions(store.as_ref());
        let (r8brain, r8brain_error) = read_saved_r8brain(store.as_ref());
        let (soxr, soxr_error) = read_saved_soxr(store.as_ref());
        let save_error = save_error
            .or(transitions_error)
            .or(r8brain_error)
            .or(soxr_error);
        let this = Self::with(executor, store, output, quality, dsd, save_error);
        this.0.state.update(|state| {
            state.settings.transitions = transitions;
            state.settings.r8brain = r8brain;
            state.written_r8brain = r8brain;
            state.settings.soxr = soxr;
            state.written_soxr = soxr;
        });
        this
    }

    fn with(
        executor: Executor,
        store: Option<config::Store>,
        output: OutputSettings,
        quality: ResamplingQuality,
        dsd: DsdOutput,
        save_error: Option<String>,
    ) -> Self {
        let r8brain = match quality {
            ResamplingQuality::R8brain(parameters) => parameters,
            _ => R8brainParameters::default(),
        };
        let soxr = match quality {
            ResamplingQuality::Soxr(parameters) => parameters,
            _ => SoxrParameters::default(),
        };
        let settings = AudioSnapshot {
            output,
            transitions: Transitions::default(),
            quality,
            rubato: match quality {
                ResamplingQuality::R8brain(_) | ResamplingQuality::Soxr(_) => {
                    ResamplingQuality::default()
                }
                quality => quality,
            },
            r8brain,
            soxr,
            dsd: dsd.clone(),
            devices: None,
            list_error: None,
            save_error,
        };
        Self(Arc::new_cyclic(|this: &Weak<Inner>| Inner {
            saver: store.map(|store| {
                let this = this.clone();
                Saver::new(executor.clone(), move |changes: Changes| {
                    let saved = write(&store, &changes);
                    if let Some(this) = this.upgrade() {
                        this.state.update(|state| {
                            state.settings.save_error = saved
                                .err()
                                .map(|error| format!("Couldn't save the audio settings: {error}."));
                        });
                    }
                })
            }),
            executor,
            state: Published::new(State {
                settings,
                written_r8brain: r8brain,
                written_soxr: soxr,
                written_dsd: dsd,
                unsaved: Unsaved::default(),
                is_listing: false,
                _list: Task::ready(()),
                #[cfg(test)]
                told: tests::Told::default(),
            }),
        }))
    }

    pub fn snapshot(&self) -> AudioSnapshot {
        self.0.state.snapshot()
    }

    /// Sees each new snapshot. A borrow of it must not be held while calling these settings,
    /// which publish under their own lock.
    pub fn subscribe(&self) -> watch::Receiver<AudioSnapshot> {
        self.0.state.subscribe()
    }

    pub fn is_exclusive_offered() -> bool {
        audio::CAN_PLAY_EXCLUSIVE
    }

    /// The choices of [`Self::set_crossfade`].
    pub fn crossfade_options() -> Vec<Choice> {
        std::iter::once(Choice::new("0", "Off"))
            .chain(
                (1..=config::LONGEST_CROSSFADE_SECONDS)
                    .map(|seconds| Choice::new(seconds.to_string(), format!("{seconds} s"))),
            )
            .collect()
    }

    /// Hands what `change` changed to the saver under the lock, so that the changes reach the
    /// disk in the order they were made.
    fn change<R>(&self, change: impl FnOnce(&mut State) -> R) -> R {
        self.0.state.update(|state| {
            let result = change(state);
            let unsaved = mem::take(&mut state.unsaved);
            if unsaved.is_any() {
                self.0.save(state, unsaved);
            }
            result
        })
    }

    pub fn set_trim_silence(&self, is_on: bool) {
        self.change(|state| state.set_trim_silence(is_on));
    }

    pub fn set_play_pause_fade(&self, is_on: bool) {
        self.change(|state| state.set_play_pause_fade(is_on));
    }

    /// `value` is one of [`Self::crossfade_options`].
    pub fn set_crossfade(&self, value: &str) {
        self.change(|state| state.set_crossfade(value));
    }

    /// Lists the devices again, unless a listing is still in progress: replacing it could keep a
    /// slow system from ever finishing one.
    pub fn list_devices(&self) {
        self.0.state.update(|state| {
            if state.is_listing {
                return;
            }
            state.is_listing = true;
            let this = Arc::downgrade(&self.0);
            state._list = self.0.executor.spawn_blocking(move || {
                let devices = audio::output_devices();
                if let Some(this) = this.upgrade() {
                    this.state.update(|state| state.on_devices_listed(devices));
                }
            });
        });
    }

    /// `value` is one of [`AudioSnapshot::device_options`].
    pub fn set_device(&self, value: &str) {
        self.change(|state| state.set_device(value));
    }

    /// `value` is one of [`AudioSnapshot::rate_options`].
    pub fn set_rate(&self, value: &str) {
        self.change(|state| state.set_rate(value));
    }

    pub fn set_exclusive(&self, is_on: bool) {
        self.change(|state| state.set_exclusive(is_on));
    }

    /// `ix` is into [`RESAMPLERS`]. Each comes back at the quality it had.
    pub fn select_resampler(&self, ix: usize) {
        self.change(|state| state.select_resampler(ix));
    }

    pub fn set_soxr_parameters(&self, parameters: SoxrParameters) {
        self.change(|state| state.set_soxr_parameters(parameters));
    }

    /// A field's change can settle after another resampler was chosen, and that one stays.
    pub fn set_r8brain_parameters(&self, parameters: R8brainParameters) {
        self.change(|state| state.set_r8brain_parameters(parameters));
    }

    /// `ix` is into [`RUBATO_QUALITIES`], and Rubato resamples at it. Custom starts from the
    /// values Rubato had, so choosing it changes nothing until one of them is changed.
    pub fn select_rubato_quality(&self, ix: usize) {
        self.change(|state| state.select_rubato_quality(ix));
    }

    /// Another size than the one in use makes the quality custom.
    pub fn set_fft_size(&self, value: &str) {
        self.change(|state| state.set_fft_size(value));
    }

    /// Another window than the one in use makes the quality custom.
    pub fn set_window(&self, value: &str) {
        self.change(|state| state.set_window(value));
    }

    pub fn set_dsd(&self, dsd: DsdOutput) {
        self.change(|state| state.set_dsd(dsd));
    }
}

impl Inner {
    fn save(&self, state: &mut State, unsaved: Unsaved) {
        let Some(saver) = &self.saver else {
            return;
        };
        let settings = &state.settings;
        let written = (
            state.written_r8brain,
            state.written_soxr,
            state.written_dsd.clone(),
        );
        saver.save_with(|waiting| {
            let (unsaved, r8brain, soxr, dsd) = match waiting {
                Some(waiting) => (
                    waiting.unsaved.and(unsaved),
                    waiting.r8brain.0,
                    waiting.soxr.0,
                    waiting.dsd.0,
                ),
                None => (unsaved, written.0, written.1, written.2),
            };
            Changes {
                unsaved,
                output: settings.output.clone(),
                transitions: settings.transitions,
                quality: settings.quality,
                r8brain: (r8brain, settings.r8brain),
                soxr: (soxr, settings.soxr),
                dsd: (dsd, settings.dsd.clone()),
            }
        });
        state.written_r8brain = state.settings.r8brain;
        state.written_soxr = state.settings.soxr;
        state.written_dsd = state.settings.dsd.clone();
    }
}

impl State {
    fn set_trim_silence(&mut self, is_on: bool) {
        if self.settings.transitions.trim_silence != is_on {
            self.settings.transitions.trim_silence = is_on;
            self.unsaved.trim_silence = true;
            self.transitions_changed();
        }
    }

    fn set_play_pause_fade(&mut self, is_on: bool) {
        if self.settings.transitions.fades_play_and_pause != is_on {
            self.settings.transitions.fades_play_and_pause = is_on;
            self.unsaved.play_pause_fade = true;
            self.transitions_changed();
        }
    }

    fn set_crossfade(&mut self, value: &str) {
        let crossfade = Duration::from_secs(value.parse().unwrap_or(0));
        if self.settings.transitions.crossfade != crossfade {
            self.settings.transitions.crossfade = crossfade;
            self.unsaved.crossfade = true;
            self.transitions_changed();
        }
    }

    fn transitions_changed(&mut self) {
        #[cfg(test)]
        {
            self.told.transitions += 1;
        }
    }

    fn on_devices_listed(&mut self, devices: Result<Vec<OutputDevice>, audio::Error>) {
        self.is_listing = false;
        let settings = &mut self.settings;
        match devices {
            Ok(devices) => {
                settings.devices = Some(devices.into());
                settings.list_error = None;
            }
            Err(error) => {
                settings.list_error = Some(format!("Couldn't list the output devices: {error}."));
            }
        }
    }

    fn set_device(&mut self, value: &str) {
        let device = (value != SYSTEM_DEFAULT).then(|| value.to_owned());
        let output = self.output_on(device, self.settings.output.sample_rate());
        if self.settings.output != output {
            self.unsaved.device = true;
            self.set_output(output);
        }
    }

    fn set_rate(&mut self, value: &str) {
        let device = self.settings.output.device().map(str::to_owned);
        let output = self.output_on(device, SampleRate::parse(value).unwrap_or_default());
        if self.settings.output != output {
            self.unsaved.rate = true;
            self.set_output(output);
        }
    }

    fn set_exclusive(&mut self, is_on: bool) {
        let output = self.settings.output.clone().with_exclusive(is_on);
        if self.settings.output != output {
            self.unsaved.exclusive = true;
            self.set_output(output);
        }
    }

    fn select_resampler(&mut self, ix: usize) {
        let quality = match ix {
            R8BRAIN_IX => ResamplingQuality::R8brain(self.settings.r8brain),
            SOXR_IX => ResamplingQuality::Soxr(self.settings.soxr),
            _ => self.settings.rubato,
        };
        self.set_quality(quality);
    }

    fn set_soxr_parameters(&mut self, parameters: SoxrParameters) {
        if self.settings.soxr == parameters {
            return;
        }
        self.settings.soxr = parameters;
        self.unsaved.soxr = true;
        if self.settings.uses_soxr() {
            self.set_quality(ResamplingQuality::Soxr(parameters));
        }
    }

    fn set_r8brain_parameters(&mut self, parameters: R8brainParameters) {
        if self.settings.r8brain == parameters {
            return;
        }
        self.settings.r8brain = parameters;
        self.unsaved.r8brain = true;
        if self.settings.uses_r8brain() {
            self.set_quality(ResamplingQuality::R8brain(parameters));
        }
    }

    fn select_rubato_quality(&mut self, ix: usize) {
        let rubato = self.settings.rubato;
        let quality = match ix {
            0 => ResamplingQuality::Medium,
            1 => ResamplingQuality::High,
            2 => ResamplingQuality::Max,
            _ => ResamplingQuality::Custom {
                fft_size: rubato.fft_size(),
                window: rubato.window(),
            },
        };
        self.set_quality(quality);
    }

    fn set_fft_size(&mut self, value: &str) {
        let Ok(fft_size) = value.parse() else {
            return;
        };
        let rubato = self.settings.rubato;
        if fft_size != rubato.fft_size() {
            let window = rubato.window();
            self.set_quality(ResamplingQuality::Custom { fft_size, window });
        }
    }

    fn set_window(&mut self, value: &str) {
        let rubato = self.settings.rubato;
        let window = ResamplingWindow::ALL
            .into_iter()
            .find(|window| window_label(*window) == value);
        if let Some(window) = window.filter(|window| *window != rubato.window()) {
            let fft_size = rubato.fft_size();
            self.set_quality(ResamplingQuality::Custom { fft_size, window });
        }
    }

    fn set_quality(&mut self, quality: ResamplingQuality) {
        if self.settings.quality == quality {
            return;
        }
        self.settings.quality = quality;
        if !matches!(
            quality,
            ResamplingQuality::R8brain(_) | ResamplingQuality::Soxr(_)
        ) {
            self.settings.rubato = quality;
        }
        self.unsaved.quality = true;
        let device = self.settings.output.device().map(str::to_owned);
        let output = self.output_on(device, self.settings.output.sample_rate());
        if self.settings.output != output {
            self.set_output(output);
        }
    }

    fn output_on(&self, device: Option<String>, rate: SampleRate) -> OutputSettings {
        let settings = &self.settings;
        OutputSettings::from_config(device, rate, settings.quality, &settings.dsd)
            .with_exclusive(settings.output.is_exclusive())
    }

    fn set_dsd(&mut self, dsd: DsdOutput) {
        if self.settings.dsd == dsd {
            return;
        }
        self.settings.dsd = dsd;
        self.unsaved.dsd = true;
        let device = self.settings.output.device().map(str::to_owned);
        let output = self.output_on(device, self.settings.output.sample_rate());
        if self.settings.output != output {
            self.set_output(output);
        }
    }

    fn set_output(&mut self, output: OutputSettings) {
        self.settings.output = output;
        #[cfg(test)]
        {
            self.told.output += 1;
        }
    }
}

/// Blocks on the disk.
fn write(store: &config::Store, changes: &Changes) -> Result<(), Box<config::Error>> {
    let Changes {
        unsaved,
        output,
        transitions,
        quality,
        r8brain,
        soxr,
        dsd,
    } = changes;
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
        store
            .save_crossfade_seconds(u32::try_from(transitions.crossfade.as_secs()).unwrap_or(0))?;
    }
    if unsaved.play_pause_fade {
        store.save_play_pause_fade(transitions.fades_play_and_pause)?;
    }
    if unsaved.r8brain {
        store.save_r8brain_changes(r8brain.0, r8brain.1)?;
    }
    if unsaved.soxr {
        store.save_soxr_changes(soxr.0, soxr.1)?;
    }
    if unsaved.quality {
        store.save_resampling_quality(*quality)?;
    }
    if unsaved.dsd {
        store.save_dsd_changes(&dsd.0, &dsd.1)?;
    }
    Ok(())
}

/// The saved output, and why it could not be read or will not be saved.
fn read_saved_output(
    store: Result<config::Store, config::Error>,
) -> (
    Option<config::Store>,
    OutputSettings,
    ResamplingQuality,
    DsdOutput,
    Option<String>,
) {
    let unread = |store, error| {
        let quality = ResamplingQuality::default();
        let dsd = DsdOutput::default();
        let output = OutputSettings::from_config(None, SampleRate::MatchSong, quality, &dsd);
        (store, output, quality, dsd, Some(error))
    };
    let store = match store {
        Ok(store) => store,
        Err(error) => {
            return unread(None, format!("Audio settings won't be saved: {error}."));
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
            let error = format!("Couldn't read the audio settings: {error}.");
            unread(Some(store), error)
        }
    }
}

/// What the file says about how the sound starts, stops and moves between songs, and why that
/// could not be read. No store means the output's own error has said so already.
fn read_saved_transitions(store: Option<&config::Store>) -> (Transitions, Option<String>) {
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
            Some(format!("Couldn't read the audio settings: {error}.")),
        ),
    }
}

/// What the file says about the r8brain resampling, whatever is in use, and why that could not
/// be read. No store means the output's own error has said so already.
fn read_saved_r8brain(store: Option<&config::Store>) -> (R8brainParameters, Option<String>) {
    let Some(store) = store else {
        return (R8brainParameters::default(), None);
    };
    match store.r8brain_parameters() {
        Ok(parameters) => (parameters, None),
        Err(error) => (
            R8brainParameters::default(),
            Some(format!("Couldn't read the audio settings: {error}.")),
        ),
    }
}

/// As [`read_saved_r8brain`]
fn read_saved_soxr(store: Option<&config::Store>) -> (SoxrParameters, Option<String>) {
    let Some(store) = store else {
        return (SoxrParameters::default(), None);
    };
    match store.soxr_parameters() {
        Ok(parameters) => (parameters, None),
        Err(error) => (
            SoxrParameters::default(),
            Some(format!("Couldn't read the audio settings: {error}.")),
        ),
    }
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

/// `devices` are `(id, name)`. A chosen device that is not among them stays an option, so the
/// choice stays visible until it is connected again.
fn device_options<'a>(
    devices: impl Iterator<Item = (&'a str, &'a str)>,
    selected: Option<&str>,
    listed: bool,
) -> Vec<Choice> {
    let mut options = vec![Choice::new(SYSTEM_DEFAULT, "System default")];
    options.extend(devices.map(|(id, name)| Choice::new(id, name)));
    if let Some(selected) = selected
        && !options.iter().any(|option| option.value() == selected)
    {
        let label = if listed {
            "Disconnected device"
        } else {
            "Loading"
        };
        options.push(Choice::new(selected, label));
    }
    options
}

/// `rates` are what the device supports, if it is listed. A multiple is offered if it fits either
/// family of rates. A chosen rate or multiple that does not fit stays an option; the engine plays
/// at what the device can do meanwhile. Without a listed device, nothing is known to be
/// unsupported.
fn rate_options(
    rates: Option<&[u32]>,
    selected: SampleRate,
    can_run_at_multiple: bool,
) -> Vec<Choice> {
    let listed = rates.unwrap_or_default();
    let fits = |multiple: u32| {
        can_run_at_multiple
            && BASE_RATES
                .iter()
                .any(|base| listed.contains(&(base * multiple)))
    };
    let multiple_text = |multiple: u32| SampleRate::SongMultiple(multiple).text();
    let rate_option = |rate: u32, label: String| Choice::new(rate.to_string(), label);
    let unsupported = |label: String| match rates {
        Some(_) => format!("{label} (not supported)"),
        None => label,
    };

    let mut options = vec![Choice::new(MATCH_SONG, "Match each song")];
    options.extend(
        SAMPLE_RATE_MULTIPLES
            .into_iter()
            .filter(|multiple| fits(*multiple))
            .map(|multiple| {
                let text = multiple_text(multiple);
                Choice::new(text.clone(), text)
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
            options.push(Choice::new(text.clone(), unsupported(text)));
        }
        _ => {}
    }
    options
}

#[cfg(test)]
pub(super) mod tests;
