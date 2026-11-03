use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::convert::Converter;
use crate::decode::Decoder;
use crate::device::device_name;
use crate::output::{self, ConfigRange};
use crate::track::Cancel;
use crate::{Error, Track};

/// Decoded audio kept ahead of the device.
const BUFFER_AHEAD: Duration = Duration::from_millis(500);
/// How often the engine thread tops up the buffer while playing.
const FILL_INTERVAL: Duration = Duration::from_millis(5);
/// How long the device keeps the rate of the last track once nothing plays.
/// Longer than opening the next track usually takes, so an album does not
/// switch the device back and forth between its songs.
const RESTORE_DELAY: Duration = Duration::from_secs(5);
/// The longest a finished track waits for the device to play its last
/// samples, in case the device stops reporting its progress.
const TAIL_TIMEOUT: Duration = Duration::from_secs(2);

/// Serializes switching device rates across engines in this process, so one
/// engine restoring a rate cannot interleave with another opening a stream.
static DEVICE_SETUP: Mutex<()> = Mutex::new(());

/// Names one [`Engine::load`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LoadId(Generation);

/// What the engine reports back. Delivered in order through the receiver
/// returned by [`Engine::spawn`].
///
/// The engine sends nothing about a load once a later `load` or `stop` has
/// been called, but events sent just before can still be in the channel.
/// Compare [`Event::load`] with the latest [`LoadId`] to drop those.
#[derive(Clone, Debug)]
pub struct Event {
    pub load: LoadId,
    pub kind: EventKind,
}

#[derive(Clone, Debug)]
pub enum EventKind {
    /// The track started playing.
    Started(PlaybackFormat),
    /// The track played to its end.
    Ended,
    /// The track stopped because of an error.
    Failed(Error),
}

/// How a track reaches the device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaybackFormat {
    device: String,
    source_rate: u32,
    source_channels: u16,
    output_rate: u32,
    output_channels: u16,
    duration: Option<Duration>,
}

impl PlaybackFormat {
    pub fn device(&self) -> &str {
        &self.device
    }

    pub fn source_rate(&self) -> u32 {
        self.source_rate
    }

    pub fn source_channels(&self) -> u16 {
        self.source_channels
    }

    pub fn output_rate(&self) -> u32 {
        self.output_rate
    }

    pub fn output_channels(&self) -> u16 {
        self.output_channels
    }

    /// Whether samples are resampled, which happens only when the device
    /// cannot run at the source rate.
    pub fn is_resampled(&self) -> bool {
        self.source_rate != self.output_rate
    }

    pub fn duration(&self) -> Option<Duration> {
        self.duration
    }
}

/// Identifies one `load` or `stop` request. Work and errors from an older
/// generation are discarded, so a superseded track can never end, fail, or
/// report a device error on behalf of the one that replaced it.
type Generation = u64;

enum Command {
    Load(Generation, Track),
    Pause,
    Resume,
    Seek(Duration),
    Stop,
    OutputFailed(Generation, String),
    Shutdown,
}

struct Shared {
    frames_played: AtomicU64,
    output_rate: AtomicU32,
    /// `f32` bits; read by the audio callback.
    volume: AtomicU32,
    /// The latest requested generation.
    generation: AtomicU64,
    /// Unblocks the current track's reads. Touched only by the engine handle
    /// and never by the audio callback.
    cancel: Mutex<Option<Cancel>>,
}

impl Shared {
    fn new() -> Self {
        Self {
            frames_played: AtomicU64::new(0),
            output_rate: AtomicU32::new(0),
            volume: AtomicU32::new(1.0f32.to_bits()),
            generation: AtomicU64::new(0),
            cancel: Mutex::new(None),
        }
    }

    fn is_current(&self, generation: Generation) -> bool {
        self.generation.load(Ordering::Acquire) == generation
    }

    /// Starts a new generation and cancels the previous track, so an engine
    /// thread blocked reading it returns to its command loop.
    fn supersede(&self, cancel: Option<Cancel>) -> Generation {
        let (generation, previous) = {
            let mut current = self.cancel.lock().unwrap_or_else(PoisonError::into_inner);
            let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
            (generation, std::mem::replace(&mut *current, cancel))
        };
        if let Some(cancel) = previous {
            cancel();
        }
        generation
    }

    /// Drops the cancel hook of a track that has finished, so the hook does
    /// not keep its source (often a whole downloaded file) alive.
    fn release(&self, generation: Generation) {
        let mut current = self.cancel.lock().unwrap_or_else(PoisonError::into_inner);
        if self.is_current(generation) {
            current.take();
        }
    }
}

/// Handle to the engine thread. Dropping it shuts the engine down; see
/// [`Engine::shutdown`].
pub struct Engine {
    commands: mpsc::Sender<Command>,
    shared: Arc<Shared>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Engine {
    pub fn spawn() -> (Self, async_channel::Receiver<Event>) {
        let (commands, command_receiver) = mpsc::channel();
        let (events, event_receiver) = async_channel::unbounded();
        let shared = Arc::new(Shared::new());
        let thread = thread::Builder::new()
            .name("hiphile-audio".to_owned())
            .spawn({
                let commands = commands.clone();
                let shared = shared.clone();
                move || run(&command_receiver, &commands, &events, &shared)
            })
            .expect("failed to spawn the audio thread");
        let engine = Self {
            commands,
            shared,
            thread: Some(thread),
        };
        (engine, event_receiver)
    }

    /// Replaces the current track and starts playing `track`. Events about
    /// it carry the returned id.
    ///
    /// The previous track gets no further events, even if it was still
    /// opening or failing at that moment.
    pub fn load(&self, mut track: Track) -> LoadId {
        let generation = self.shared.supersede(track.cancel.take());
        self.send(Command::Load(generation, track));
        LoadId(generation)
    }

    /// Takes effect once the engine thread is not waiting for the track's
    /// data; [`Self::stop`] and [`Self::load`] do not have to wait.
    pub fn pause(&self) {
        self.send(Command::Pause);
    }

    pub fn resume(&self) {
        self.send(Command::Resume);
    }

    /// Jumps within the current track. Keeps it paused if it is.
    ///
    /// The position reads `to` right away and the old audio goes silent; the
    /// new audio starts once the track's data at `to` is available, which
    /// for a download may mean waiting for it to arrive. A track that cannot
    /// seek fails with [`EventKind::Failed`].
    pub fn seek(&self, to: Duration) {
        self.send(Command::Seek(to));
    }

    pub fn stop(&self) {
        self.shared.supersede(None);
        self.send(Command::Stop);
    }

    /// Linear gain from `0.0` to `1.0`, applied immediately. At `1.0`
    /// samples reach the device unchanged; NaN counts as silence.
    pub fn set_volume(&self, volume: f32) {
        let volume = if volume.is_nan() {
            0.0
        } else {
            volume.clamp(0.0, 1.0)
        };
        self.shared
            .volume
            .store(volume.to_bits(), Ordering::Relaxed);
    }

    /// How much of the current track the device has played.
    pub fn position(&self) -> Duration {
        let rate = self.shared.output_rate.load(Ordering::Relaxed);
        if rate == 0 {
            return Duration::ZERO;
        }
        let frames = self.shared.frames_played.load(Ordering::Relaxed);
        Duration::from_secs_f64(frames as f64 / f64::from(rate))
    }

    /// Stops playback, returns the device to the sample rate it had before
    /// the engine changed it, and waits for the engine thread to finish.
    /// Switching the rate can take a second or more on some devices. Later
    /// calls do nothing, and the engine ignores everything after.
    ///
    /// Use it where the process may exit right after; otherwise dropping the
    /// engine does the same without waiting.
    pub fn shutdown(&mut self) {
        if let Some(thread) = self.stop_thread() {
            thread.join().ok();
        }
    }

    fn stop_thread(&mut self) -> Option<thread::JoinHandle<()>> {
        let thread = self.thread.take()?;
        // Unblock a read first, or the thread could not see `Shutdown`.
        self.shared.supersede(None);
        self.send(Command::Shutdown);
        Some(thread)
    }

    fn send(&self, command: Command) {
        // The thread exits only on shutdown; later commands have no effect.
        self.commands.send(command).ok();
    }
}

impl Drop for Engine {
    /// Stops playback without waiting: the engine thread restores the
    /// device's rate and exits on its own.
    fn drop(&mut self) {
        self.stop_thread();
    }
}

fn run(
    commands: &mpsc::Receiver<Command>,
    command_sender: &mpsc::Sender<Command>,
    events: &async_channel::Sender<Event>,
    shared: &Arc<Shared>,
) {
    let mut restore = None;
    serve(commands, command_sender, events, shared, &mut restore);
    // `serve` has closed the last stream, so the device is free to switch.
    if let Some(restore) = restore {
        restore.apply();
    }
}

fn serve(
    commands: &mpsc::Receiver<Command>,
    command_sender: &mpsc::Sender<Command>,
    events: &async_channel::Sender<Event>,
    shared: &Arc<Shared>,
    restore: &mut Option<RateRestore>,
) {
    let emit = |generation, kind| {
        let load = LoadId(generation);
        events.send_blocking(Event { load, kind }).ok();
    };
    let mut current: Option<Playback> = None;
    let mut idle_since: Option<Instant> = None;
    // A command that arrived behind a run of seeks.
    let mut deferred: Option<Command> = None;
    loop {
        if current.is_some() {
            idle_since = None;
        } else {
            idle_since.get_or_insert_with(Instant::now);
        }
        let playing = current.as_ref().is_some_and(|playback| !playback.paused);
        let timeout = if playing {
            Some(FILL_INTERVAL)
        } else {
            idle_since
                .filter(|_| restore.is_some())
                .map(|since| RESTORE_DELAY.saturating_sub(since.elapsed()))
        };
        let command = match (deferred.take(), timeout) {
            (Some(command), _) => Some(command),
            (None, Some(timeout)) => match commands.recv_timeout(timeout) {
                Ok(command) => Some(command),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            },
            (None, None) => match commands.recv() {
                Ok(command) => Some(command),
                Err(_) => return,
            },
        };
        // Each seek may wait on the network; of seeks queued meanwhile, only
        // the last matters.
        let command = match command {
            Some(Command::Seek(mut to)) => {
                while let Ok(next) = commands.try_recv() {
                    match next {
                        Command::Seek(later) => to = later,
                        other => {
                            deferred = Some(other);
                            break;
                        }
                    }
                }
                Some(Command::Seek(to))
            }
            command => command,
        };
        if command.is_none() && current.is_none() {
            // Nothing has played for `RESTORE_DELAY`; no next track is coming.
            if let Some(restore) = restore.take() {
                restore.apply();
            }
            continue;
        }

        // An error names the generation it ends.
        let result: Result<(), (Generation, Error)> = match command {
            None => Ok(()),
            Some(Command::Load(generation, track)) => {
                // Release the device before opening it again.
                current = None;
                // A newer request already replaced this one; dropping the
                // track releases its source.
                if shared.is_current(generation) {
                    match Playback::start(track, generation, shared, command_sender, restore) {
                        Ok(playback) if shared.is_current(generation) => {
                            emit(generation, EventKind::Started(playback.format.clone()));
                            current = Some(playback);
                            Ok(())
                        }
                        Ok(_) => Ok(()),
                        Err(_) if !shared.is_current(generation) => Ok(()),
                        Err(error) => {
                            shared.release(generation);
                            Err((generation, error))
                        }
                    }
                } else {
                    Ok(())
                }
            }
            Some(Command::Pause) => current.as_mut().map_or(Ok(()), |playback| {
                playback
                    .pause()
                    .map_err(|error| (playback.generation, error))
            }),
            Some(Command::Resume) => current.as_mut().map_or(Ok(()), |playback| {
                playback
                    .resume()
                    .map_err(|error| (playback.generation, error))
            }),
            Some(Command::Seek(to)) => current.as_mut().map_or(Ok(()), |playback| {
                playback
                    .seek(to, shared)
                    .map_err(|error| (playback.generation, error))
            }),
            Some(Command::Stop) => {
                current = None;
                Ok(())
            }
            Some(Command::OutputFailed(generation, message)) => {
                let affects_current = current
                    .as_ref()
                    .is_some_and(|playback| playback.generation == generation);
                if affects_current {
                    Err((generation, Error::Output(message)))
                } else {
                    Ok(())
                }
            }
            Some(Command::Shutdown) => return,
        };
        if let Err((generation, error)) = result {
            if let Some(playback) = current.take() {
                shared.release(playback.generation);
            }
            // A superseded track was canceled on purpose; stay silent.
            if shared.is_current(generation) {
                emit(generation, EventKind::Failed(error));
            }
            continue;
        }

        if let Some(playback) = current.as_mut().filter(|playback| !playback.paused) {
            let generation = playback.generation;
            let fill = playback.fill();
            if !matches!(fill, Ok(Fill::Playing)) {
                current = None;
            }
            // A superseded track was canceled on purpose; stay silent.
            if !shared.is_current(generation) {
                continue;
            }
            if !matches!(fill, Ok(Fill::Playing)) {
                shared.release(generation);
            }
            match fill {
                Ok(Fill::Playing) => {}
                Ok(Fill::Drained) => emit(generation, EventKind::Ended),
                Err(error) => emit(generation, EventKind::Failed(error)),
            }
        }
    }
}

/// Whether a stream error ends playback. cpal reports some conditions on the
/// error callback while the stream keeps running: an overload (`Xrun`) on
/// every glitch, a reroute of the default device, or a denied real-time
/// priority. Treating those as failures would stop a track over a click.
fn is_fatal(kind: cpal::ErrorKind) -> bool {
    use cpal::ErrorKind::*;
    !matches!(kind, Xrun | DeviceChanged | RealtimeDenied)
}

enum Fill {
    Playing,
    /// The device has played every sample of the track.
    Drained,
}

/// What the engine needs to switch a device back to the sample rate it had
/// before the engine first changed it.
///
/// On macOS the rate a stream asks for becomes the device's rate for every
/// app, and it stays after the stream closes. Other platforms either convert
/// per stream or release the rate with the stream, so nothing is recorded
/// there.
///
/// Only the rate comes back. cpal may also change the device's physical
/// format, such as 24-bit integer to 32-bit float, and has no way to restore
/// that.
struct RateRecord<D> {
    device: D,
    /// The device's configuration before the first switch.
    original: cpal::SupportedStreamConfig,
    /// The rate the engine last set. A different rate means someone else has
    /// changed it since, and theirs is kept.
    switched_to: u32,
}

type RateRestore = RateRecord<cpal::DeviceId>;

impl<D: PartialEq> RateRecord<D> {
    /// Takes the record if it is for a device other than `device`: the engine
    /// has moved off it, so it should be restored now.
    fn take_if_other(slot: &mut Option<Self>, device: Option<&D>) -> Option<Self> {
        if &slot.as_ref()?.device == device? {
            None
        } else {
            slot.take()
        }
    }

    /// Notes that a stream at `rate` is opening on `device`, whose
    /// configuration is `current`. Starts a record on the first switch away
    /// from the device's rate and keeps its original rate after that.
    fn note(slot: &mut Option<Self>, device: D, current: cpal::SupportedStreamConfig, rate: u32) {
        match slot {
            Some(record) if record.device == device => record.switched_to = rate,
            _ if rate != current.sample_rate() => {
                *slot = Some(Self {
                    device,
                    original: current,
                    switched_to: rate,
                });
            }
            _ => {}
        }
    }

    /// Whether a device now running at `current_rate` should be switched
    /// back.
    fn needs_restore(&self, current_rate: u32) -> bool {
        current_rate == self.switched_to && current_rate != self.original.sample_rate()
    }
}

impl RateRestore {
    /// Switches the device back, unless someone else has changed its rate
    /// since. Call only while the engine has no stream on it. Best effort: a
    /// device that is gone or refuses keeps its rate.
    fn apply(self) {
        let _setup = DEVICE_SETUP.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(device) = cpal::default_host().device_by_id(&self.device) else {
            return;
        };
        let Ok(current) = device.default_output_config() else {
            return;
        };
        if !self.needs_restore(current.sample_rate()) {
            return;
        }
        // cpal sets a device to a stream's rate when it builds the stream, so
        // building one at the old rate switches the device back. The stream
        // never starts and closes right away.
        let stream = device.build_output_stream_raw(
            self.original.config(),
            self.original.sample_format(),
            |_: &mut cpal::Data, _: &cpal::OutputCallbackInfo| {},
            |_| {},
            None,
        );
        drop(stream);
    }
}

/// One track playing on one device stream. Lives on the engine thread only;
/// `cpal::Stream` is not `Send` on every platform. A new stream is opened per
/// track so the device can follow each file's sample rate.
struct Playback {
    decoder: Decoder,
    converter: Converter,
    producer: rtrb::Producer<f32>,
    capacity: usize,
    /// Converted samples not yet in the ring buffer.
    pending: Vec<f32>,
    pending_ix: usize,
    decoded: Vec<f32>,
    finished: bool,
    paused: bool,
    format: PlaybackFormat,
    generation: Generation,
    /// Set by the audio callback while samples it handed to the device have
    /// not all been played.
    sounding: Arc<AtomicBool>,
    /// When the ring buffer ran empty after the last sample was decoded.
    emptied_at: Option<Instant>,
    /// Samples ever written to the ring buffer.
    written: u64,
    /// The audio callback drops samples numbered below this instead of
    /// playing them: what was buffered before a seek.
    flush_until: Arc<AtomicU64>,
    stream: cpal::Stream,
}

impl Playback {
    fn start(
        track: Track,
        generation: Generation,
        shared: &Arc<Shared>,
        commands: &mpsc::Sender<Command>,
        restore: &mut Option<RateRestore>,
    ) -> Result<Self, Error> {
        let decoder = Decoder::open(track)?;
        let source = decoder.spec();

        let device = cpal::default_host()
            .default_output_device()
            .ok_or(Error::NoOutputDevice)?;
        let device_id = cfg!(target_os = "macos")
            .then(|| device.id().ok())
            .flatten();
        if let Some(left) = RateRestore::take_if_other(restore, device_id.as_ref()) {
            left.apply();
        }
        let ranges: Vec<ConfigRange> = device
            .supported_output_configs()?
            .filter(|range| range.sample_format() == cpal::SampleFormat::F32)
            .map(|range| ConfigRange {
                channels: range.channels(),
                min_rate: range.min_sample_rate(),
                max_rate: range.max_sample_rate(),
            })
            .collect();
        // Held until the stream exists, which is when cpal switches the rate.
        let setup = DEVICE_SETUP.lock().unwrap_or_else(PoisonError::into_inner);
        let device_config = device.default_output_config()?;
        let device_rate = device_config.sample_rate();
        let output =
            output::plan(&ranges, device_rate, source).ok_or(Error::UnsupportedOutputFormat)?;
        if let Some(device_id) = device_id {
            RateRestore::note(restore, device_id, device_config, output.rate);
        }
        let config = cpal::StreamConfig {
            channels: output.channels,
            sample_rate: output.rate,
            buffer_size: cpal::BufferSize::Default,
        };

        let channels = usize::from(output.channels);
        let ahead_frames = (output.rate as f64 * BUFFER_AHEAD.as_secs_f64()) as usize;
        let capacity = ahead_frames.max(1) * channels;
        let (producer, consumer) = rtrb::RingBuffer::new(capacity);

        shared.frames_played.store(0, Ordering::Relaxed);
        shared.output_rate.store(output.rate, Ordering::Relaxed);
        let sounding = Arc::new(AtomicBool::new(false));
        let flush_until = Arc::new(AtomicU64::new(0));
        let stream = device.build_output_stream(
            config,
            output_callback(
                consumer,
                channels,
                output.rate,
                Arc::clone(shared),
                Arc::clone(&sounding),
                Arc::clone(&flush_until),
            ),
            {
                let commands = commands.clone();
                move |error: cpal::Error| {
                    if is_fatal(error.kind()) {
                        commands
                            .send(Command::OutputFailed(generation, error.to_string()))
                            .ok();
                    }
                }
            },
            None,
        )?;
        drop(setup);

        let mut playback = Self {
            converter: Converter::new(source, output)?,
            format: PlaybackFormat {
                device: device_name(&device),
                source_rate: source.rate,
                source_channels: source.channels,
                output_rate: output.rate,
                output_channels: output.channels,
                duration: decoder.duration(),
            },
            decoder,
            producer,
            capacity,
            pending: Vec::new(),
            pending_ix: 0,
            decoded: Vec::new(),
            finished: false,
            paused: false,
            generation,
            sounding,
            emptied_at: None,
            written: 0,
            flush_until,
            stream,
        };
        // Start with a full buffer so the first callbacks are not silent.
        playback.fill()?;
        playback.stream.play()?;
        Ok(playback)
    }

    fn pause(&mut self) -> Result<(), Error> {
        self.stream.pause()?;
        self.paused = true;
        Ok(())
    }

    fn resume(&mut self) -> Result<(), Error> {
        self.stream.play()?;
        self.paused = false;
        Ok(())
    }

    fn seek(&mut self, to: Duration, shared: &Shared) -> Result<(), Error> {
        let rate = f64::from(self.format.output_rate);
        let frames = |time: Duration| (time.as_secs_f64() * rate) as u64;
        // Silence what is buffered at once; the seek may wait for data.
        self.flush_until.store(self.written, Ordering::Release);
        shared.frames_played.store(frames(to), Ordering::Relaxed);
        let resumed = self.decoder.seek(to)?;
        if let Some(resumed) = resumed {
            shared
                .frames_played
                .store(frames(resumed), Ordering::Relaxed);
        }
        self.converter.reset()?;
        self.pending.clear();
        self.pending_ix = 0;
        // Past the end, the track ends once the device has played out.
        self.finished = resumed.is_none();
        self.emptied_at = None;
        Ok(())
    }

    /// Tops the ring buffer up; returns whether the track has fully played.
    fn fill(&mut self) -> Result<Fill, Error> {
        loop {
            if self.pending_ix < self.pending.len() {
                let free = self.producer.slots();
                if free == 0 {
                    return Ok(Fill::Playing);
                }
                let samples = &self.pending[self.pending_ix..];
                let len = free.min(samples.len());
                let chunk = self
                    .producer
                    .write_chunk_uninit(len)
                    .expect("`len` is at most the free slots");
                let written = chunk.fill_from_iter(samples[..len].iter().copied());
                self.pending_ix += written;
                self.written += written as u64;
                continue;
            }
            if self.finished {
                if self.producer.slots() < self.capacity {
                    return Ok(Fill::Playing);
                }
                // The device has taken every sample but may still be playing
                // the last ones; closing the stream now would cut them off.
                let emptied_at = *self.emptied_at.get_or_insert_with(Instant::now);
                let played =
                    !self.sounding.load(Ordering::Acquire) || emptied_at.elapsed() >= TAIL_TIMEOUT;
                return Ok(if played { Fill::Drained } else { Fill::Playing });
            }
            self.pending.clear();
            self.pending_ix = 0;
            self.decoded.clear();
            if self.decoder.decode_into(&mut self.decoded)? {
                self.converter.process(&self.decoded, &mut self.pending)?;
            } else {
                self.converter.finish(&mut self.pending)?;
                self.finished = true;
            }
        }
    }
}

/// The real-time callback: copies whole frames out of the ring buffer and
/// fills any shortfall with silence. No allocation, locking, or blocking.
///
/// `sounding` is true from the moment samples go to the device until a later
/// callback starts after the device's predicted time for playing the last of
/// them. Samples numbered below `flush_until` are dropped unplayed.
fn output_callback(
    mut consumer: rtrb::Consumer<f32>,
    channels: usize,
    rate: u32,
    shared: Arc<Shared>,
    sounding: Arc<AtomicBool>,
    flush_until: Arc<AtomicU64>,
) -> impl FnMut(&mut [f32], &cpal::OutputCallbackInfo) + Send + 'static {
    // When the device will have played the last sample written so far.
    let mut played_by: Option<cpal::StreamInstant> = None;
    // Samples taken out of the ring buffer, played or dropped.
    let mut read: u64 = 0;
    move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
        let stale = flush_until.load(Ordering::Acquire).saturating_sub(read);
        if stale > 0 {
            let len = usize::try_from(stale)
                .unwrap_or(usize::MAX)
                .min(consumer.slots());
            if let Ok(chunk) = consumer.read_chunk(len) {
                chunk.commit_all();
                read += len as u64;
            }
        }
        let timestamp = info.timestamp();
        let available = consumer.slots().min(data.len());
        // Whole frames only, so an underrun never shifts the channel order.
        let len = available - available % channels;
        let mut written = 0;
        if let Ok(chunk) = consumer.read_chunk(len) {
            let (first, second) = chunk.as_slices();
            data[..first.len()].copy_from_slice(first);
            data[first.len()..first.len() + second.len()].copy_from_slice(second);
            written = first.len() + second.len();
            read += written as u64;
            if written > 0 {
                let frames = (written / channels) as f64;
                played_by = timestamp
                    .playback
                    .checked_add(Duration::from_secs_f64(frames / f64::from(rate)));
                // Before the commit: whoever sees the ring buffer empty must
                // also see these samples as still sounding.
                sounding.store(true, Ordering::Release);
            }
            chunk.commit_all();
        }
        if written == 0 && played_by.is_some_and(|played_by| timestamp.callback >= played_by) {
            played_by = None;
            sounding.store(false, Ordering::Release);
        }
        let volume = f32::from_bits(shared.volume.load(Ordering::Relaxed));
        if volume != 1.0 {
            for sample in &mut data[..written] {
                *sample *= volume;
            }
        }
        data[written..].fill(0.0);
        shared
            .frames_played
            .fetch_add((written / channels) as u64, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::time::Instant;

    use super::*;
    use crate::test_wav::wav;

    fn next_event(events: &async_channel::Receiver<Event>) -> Event {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match events.try_recv() {
                Ok(event) => return event,
                Err(async_channel::TryRecvError::Empty) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("no event: {error:?}"),
            }
        }
    }

    /// Plays silence on the default device: Started, then Ended once the
    /// device has consumed every frame.
    #[test]
    #[ignore = "needs an audio output device"]
    fn plays_a_track_to_its_end_on_the_default_device() {
        let (engine, events) = Engine::spawn();
        let started = Instant::now();
        engine.load(Track::new(Cursor::new(wav(44_100, 2, 22_050, 0))));
        let EventKind::Started(format) = next_event(&events).kind else {
            panic!("expected Started");
        };
        assert_eq!(format.source_rate(), 44_100);
        assert!(matches!(next_event(&events).kind, EventKind::Ended));
        assert!(started.elapsed() >= Duration::from_millis(450));
        assert!(engine.position() >= Duration::from_millis(450));
    }

    #[test]
    #[ignore = "needs an audio output device"]
    fn a_seek_past_the_end_ends_the_track() {
        let (engine, events) = Engine::spawn();
        let rate = default_device_rate();
        engine.load(Track::new(Cursor::new(wav(rate, 2, rate * 10, 0))));
        assert!(matches!(next_event(&events).kind, EventKind::Started(_)));
        let sought = Instant::now();
        engine.seek(Duration::from_secs(20));
        assert!(matches!(next_event(&events).kind, EventKind::Ended));
        assert!(sought.elapsed() < Duration::from_secs(5));
    }

    /// 384 kHz is beyond typical devices, so this exercises resampling.
    #[test]
    #[ignore = "needs an audio output device"]
    fn resamples_rates_the_device_cannot_run_at() {
        let (engine, events) = Engine::spawn();
        engine.load(Track::new(Cursor::new(wav(384_000, 2, 96_000, 0))));
        let EventKind::Started(format) = next_event(&events).kind else {
            panic!("expected Started");
        };
        assert!(format.is_resampled(), "{format:?}");
        assert!(matches!(next_event(&events).kind, EventKind::Ended));
    }

    fn default_device_rate() -> u32 {
        let device = cpal::default_host().default_output_device().unwrap();
        device.default_output_config().unwrap().sample_rate()
    }

    /// Plays a short track at an F32 rate the default device supports but is
    /// not running at, and returns the device's rate from before.
    fn play_at_another_rate(engine: &Engine, events: &async_channel::Receiver<Event>) -> u32 {
        let device = cpal::default_host().default_output_device().unwrap();
        let original = default_device_rate();
        let other = [44_100, 48_000, 96_000]
            .into_iter()
            .find(|rate| {
                *rate != original
                    && device.supported_output_configs().unwrap().any(|range| {
                        range.sample_format() == cpal::SampleFormat::F32
                            && (range.min_sample_rate()..=range.max_sample_rate()).contains(rate)
                    })
            })
            .expect("no second rate to switch to");
        engine.load(Track::new(Cursor::new(wav(other, 2, other / 10, 0))));
        let EventKind::Started(format) = next_event(events).kind else {
            panic!("expected Started");
        };
        assert_eq!(format.output_rate(), other);
        assert_eq!(default_device_rate(), other);
        assert!(matches!(next_event(events).kind, EventKind::Ended));
        original
    }

    #[test]
    #[ignore = "needs an audio output device with two F32 rates; run with --test-threads=1"]
    #[cfg(target_os = "macos")]
    fn shutdown_restores_the_device_rate() {
        let (mut engine, events) = Engine::spawn();
        let original = play_at_another_rate(&engine, &events);
        engine.shutdown();
        assert_eq!(default_device_rate(), original);
    }

    #[test]
    #[ignore = "needs an audio output device with two F32 rates; run with --test-threads=1"]
    #[cfg(target_os = "macos")]
    fn an_idle_engine_restores_the_device_rate() {
        let (engine, events) = Engine::spawn();
        let original = play_at_another_rate(&engine, &events);
        thread::sleep(RESTORE_DELAY - Duration::from_secs(1));
        assert_ne!(default_device_rate(), original, "restored too early");
        thread::sleep(Duration::from_secs(2));
        assert_eq!(default_device_rate(), original);
    }

    fn config(rate: u32) -> cpal::SupportedStreamConfig {
        cpal::SupportedStreamConfig::new(
            2,
            rate,
            cpal::SupportedBufferSize::Unknown,
            cpal::SampleFormat::F32,
        )
    }

    #[test]
    fn a_rate_record_starts_at_the_first_switch() {
        let mut slot = None;
        RateRecord::note(&mut slot, "speakers", config(44_100), 44_100);
        assert!(slot.is_none(), "nothing switched");
        RateRecord::note(&mut slot, "speakers", config(44_100), 96_000);
        RateRecord::note(&mut slot, "speakers", config(96_000), 48_000);
        let record = slot.unwrap();
        assert_eq!(record.original.sample_rate(), 44_100);
        assert_eq!(record.switched_to, 48_000);
    }

    #[test]
    fn a_rate_record_for_a_device_left_behind_is_taken() {
        let mut slot = None;
        RateRecord::note(&mut slot, "speakers", config(44_100), 96_000);
        assert!(RateRecord::take_if_other(&mut slot, Some(&"speakers")).is_none());
        assert!(RateRecord::take_if_other(&mut slot, None).is_none());
        // The next device plays at its own rate; the old one still goes back.
        let left = RateRecord::take_if_other(&mut slot, Some(&"headphones")).unwrap();
        assert_eq!(left.device, "speakers");
        assert!(slot.is_none());
    }

    #[test]
    fn a_rate_changed_by_someone_else_is_kept() {
        let mut slot = None;
        RateRecord::note(&mut slot, "speakers", config(44_100), 96_000);
        let record = slot.unwrap();
        assert!(record.needs_restore(96_000));
        assert!(!record.needs_restore(88_200), "changed since");
        assert!(!record.needs_restore(44_100), "already back");
    }

    fn callback_info(callback_ms: u64, latency_ms: u64) -> cpal::OutputCallbackInfo {
        cpal::OutputCallbackInfo::new(cpal::OutputStreamTimestamp {
            callback: cpal::StreamInstant::from_millis(callback_ms),
            playback: cpal::StreamInstant::from_millis(callback_ms + latency_ms),
        })
    }

    /// The last samples count as sounding until the device's predicted
    /// playback time for them has passed, not just until they leave the ring.
    #[test]
    fn samples_sound_until_the_device_has_played_them() {
        let (mut producer, consumer) = rtrb::RingBuffer::new(64);
        let shared = Arc::new(Shared::new());
        let sounding = Arc::new(AtomicBool::new(false));
        // 1 kHz stereo: one frame per millisecond.
        let mut callback = output_callback(
            consumer,
            2,
            1_000,
            shared.clone(),
            sounding.clone(),
            Arc::default(),
        );
        for sample in [0.5; 8] {
            producer.push(sample).unwrap();
        }
        let mut data = [1.0; 16];

        // Four frames go out at 0 ms and play from 10 ms, so until 14 ms.
        callback(&mut data, &callback_info(0, 10));
        assert_eq!(data[..8], [0.5; 8]);
        assert_eq!(data[8..], [0.0; 8]);
        assert_eq!(shared.frames_played.load(Ordering::Relaxed), 4);
        assert_eq!(producer.slots(), 64);
        assert!(sounding.load(Ordering::Acquire));

        callback(&mut data, &callback_info(8, 10));
        assert!(sounding.load(Ordering::Acquire), "still in the device");
        callback(&mut data, &callback_info(14, 10));
        assert!(!sounding.load(Ordering::Acquire));
        assert_eq!(data, [0.0; 16]);
    }

    /// After a seek, what was buffered before it is dropped, not played.
    #[test]
    fn samples_before_a_seek_are_dropped() {
        let (mut producer, consumer) = rtrb::RingBuffer::new(64);
        let shared = Arc::new(Shared::new());
        let flush_until = Arc::new(AtomicU64::new(0));
        let mut callback = output_callback(
            consumer,
            2,
            1_000,
            shared.clone(),
            Arc::default(),
            flush_until.clone(),
        );
        for sample in [0.1; 6] {
            producer.push(sample).unwrap();
        }
        let mut data = [1.0; 2];
        callback(&mut data, &callback_info(0, 10));
        assert_eq!(data, [0.1, 0.1]);

        // Seek: six samples were written so far; the next two are new.
        flush_until.store(6, Ordering::Release);
        for sample in [0.9; 2] {
            producer.push(sample).unwrap();
        }
        let mut data = [1.0; 4];
        callback(&mut data, &callback_info(2, 10));
        assert_eq!(data, [0.9, 0.9, 0.0, 0.0]);
        assert_eq!(shared.frames_played.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn an_underrun_keeps_whole_frames() {
        let (mut producer, consumer) = rtrb::RingBuffer::new(64);
        let shared = Arc::new(Shared::new());
        let sounding = Arc::new(AtomicBool::new(false));
        let mut callback =
            output_callback(consumer, 2, 1_000, shared.clone(), sounding, Arc::default());
        for sample in [0.1, 0.2, 0.3] {
            producer.push(sample).unwrap();
        }
        let mut data = [1.0; 4];
        callback(&mut data, &callback_info(0, 10));
        assert_eq!(data, [0.1, 0.2, 0.0, 0.0]);
        assert_eq!(shared.frames_played.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn only_stream_breaking_errors_are_fatal() {
        use cpal::ErrorKind::*;
        for kind in [Xrun, DeviceChanged, RealtimeDenied] {
            assert!(!is_fatal(kind), "{kind:?}");
        }
        for kind in [StreamInvalidated, DeviceNotAvailable, BackendError, Other] {
            assert!(is_fatal(kind), "{kind:?}");
        }
    }

    /// A source that never has data until canceled, like a stalled download.
    fn stalled_track() -> Track {
        #[derive(Default)]
        struct Gate {
            canceled: std::sync::Mutex<bool>,
            changed: std::sync::Condvar,
        }
        struct Stalled(Arc<Gate>);
        impl std::io::Read for Stalled {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                let mut canceled = self.0.canceled.lock().unwrap();
                while !*canceled {
                    canceled = self.0.changed.wait(canceled).unwrap();
                }
                Err(std::io::Error::other("canceled"))
            }
        }
        impl std::io::Seek for Stalled {
            fn seek(&mut self, _: std::io::SeekFrom) -> std::io::Result<u64> {
                Ok(0)
            }
        }
        let gate = Arc::new(Gate::default());
        Track::new(Stalled(gate.clone())).cancel_with(move || {
            *gate.canceled.lock().unwrap() = true;
            gate.changed.notify_all();
        })
    }

    fn assert_no_event(events: &async_channel::Receiver<Event>) {
        thread::sleep(Duration::from_millis(300));
        assert!(events.try_recv().is_err(), "unexpected event");
    }

    /// Shuts the engine down on another thread and fails if the engine
    /// thread does not finish.
    fn shutdown_within_deadline(mut engine: Engine) {
        let (done, finished) = mpsc::channel();
        thread::spawn(move || {
            engine.shutdown();
            done.send(()).ok();
        });
        finished
            .recv_timeout(Duration::from_secs(5))
            .expect("shutting the engine down hung");
    }

    #[test]
    fn stop_unblocks_a_stalled_track_and_reports_nothing() {
        let (engine, events) = Engine::spawn();
        engine.load(stalled_track());
        thread::sleep(Duration::from_millis(100));
        engine.stop();
        assert_no_event(&events);
        shutdown_within_deadline(engine);
    }

    #[test]
    fn a_replaced_track_reports_nothing() {
        let (engine, events) = Engine::spawn();
        engine.load(stalled_track());
        thread::sleep(Duration::from_millis(100));
        let replacement = engine.load(Track::new(Cursor::new(b"not audio".to_vec())));
        // Exactly one failure: the replacement's, not the canceled track's.
        let event = next_event(&events);
        assert_eq!(event.load, replacement);
        assert!(matches!(event.kind, EventKind::Failed(_)));
        assert_no_event(&events);
    }

    #[test]
    fn dropping_the_engine_unblocks_a_stalled_track() {
        let (engine, _events) = Engine::spawn();
        engine.load(stalled_track());
        thread::sleep(Duration::from_millis(100));
        shutdown_within_deadline(engine);
    }

    #[test]
    fn reports_undecodable_tracks() {
        let (engine, events) = Engine::spawn();
        engine.load(Track::new(Cursor::new(b"not audio".to_vec())));
        assert!(matches!(next_event(&events).kind, EventKind::Failed(_)));
    }
}
