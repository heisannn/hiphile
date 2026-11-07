use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait};

use crate::convert::Converter;
use crate::device::device_name;
use crate::exclusive_device::{Exclusive, OriginalFormat};
use crate::feed::{Feed, Item, Opening, Polled};
use crate::output::{self, ConfigRange, OutputSettings, Spec};
use crate::stream::Stream;
use crate::track::Cancel;
use crate::{Error, Track};

/// Audio kept in the ring buffer, ahead of the device. More waits decoded in
/// the track's [`Feed`].
const BUFFER_AHEAD: Duration = Duration::from_millis(500);
/// How much of a track must be ready before it sounds. Each time it then runs
/// out, as its data comes too slowly, it waits for twice as much, up to
/// [`LONGEST_HOLD`], so that the gaps get rarer instead of the sound choppy.
const FIRST_HOLD: Duration = Duration::from_secs(2);
const LONGEST_HOLD: Duration = Duration::from_secs(30);
/// How long a track must wait for its data for that to be reported. Most
/// waits are over before anyone could tell.
const BUFFERING_NOTICE: Duration = Duration::from_millis(300);
const FILL_INTERVAL: Duration = Duration::from_millis(5);
/// How long the device keeps the rate of the last track once nothing plays.
/// Longer than opening the next track usually takes, so an album does not
/// switch the device back and forth between its songs.
const RESTORE_DELAY: Duration = Duration::from_secs(5);
/// The longest a finished track waits for the device to play its last
/// samples, in case the device stops reporting its progress.
const TAIL_TIMEOUT: Duration = Duration::from_secs(2);

/// How long opening a stream waits for the device to reach a new sample rate.
/// External DACs can take well over cpal's default of a second, and a stream
/// that gives up leaves the device switching anyway.
const RATE_SWITCH_TIMEOUT: Duration = Duration::from_secs(5);

/// Silence before the first DoP frames of a stream, since a DAC takes a moment
/// to lock onto DoP and whatever comes before that is lost, and no device
/// reports when it has. A FiiO K9 at DSD256 still lost a few hundred
/// milliseconds after 250 ms.
const DOP_LEAD_IN: Duration = Duration::from_secs(1);
/// How long the sound rises after DoP takes over from PCM, whose volume may
/// have been far below full level.
const DOP_FADE_IN: Duration = Duration::from_secs(3);
const DOP_CHANNELS: u16 = 2;

/// Serializes switching device rates across engines in this process, so one
/// engine restoring a rate cannot interleave with another opening a stream.
static DEVICE_SETUP: Mutex<()> = Mutex::new(());

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
    Started(PlaybackFormat),
    /// The track moved to other output settings, or to the rate someone else
    /// set on the device, and goes on from where it was, paused if it was.
    OutputChanged(PlaybackFormat),
    /// The track played to its end. `next` is the [`Engine::set_next`] load
    /// that goes on from here and reports [`EventKind::Started`] or
    /// [`EventKind::Failed`] next, perhaps after [`EventKind::Buffering`]; with
    /// `None`, nothing else plays.
    Ended {
        next: Option<LoadId>,
    },
    /// The track has been silent for a while waiting for its data, while
    /// opening or playing, or has stopped doing so. Not reported while paused.
    Buffering(bool),
    Failed(Error),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaybackFormat {
    device: String,
    source_rate: u32,
    source_channels: u16,
    output_rate: u32,
    output_channels: u16,
    dop: Option<dsd::DopRate>,
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

    /// The DSD rate, when the output is DoP. [`Self::output_rate`] is then the
    /// rate of the frames.
    pub fn dop_rate(&self) -> Option<dsd::DopRate> {
        self.dop
    }

    /// PCM going to the device at another rate. DoP is not counted, as it
    /// always is converted.
    pub fn is_resampled(&self) -> bool {
        self.dop.is_none() && self.source_rate != self.output_rate
    }

    pub fn duration(&self) -> Option<Duration> {
        self.duration
    }
}

/// Identifies one `load`, `set_next` or `stop` request. Work and errors from
/// a generation that is not current are discarded, so a superseded track can
/// never end, fail, or report a device error on behalf of the one that
/// replaced it.
type Generation = u64;

enum Command {
    Load(Generation, Track),
    SetNext(Generation, Track),
    ClearNext,
    Pause,
    Resume,
    Seek(Duration),
    SetOutput(OutputSettings),
    Stop,
    /// The stream stopped because the device changed under it, such as its
    /// rate being set from outside the engine. Names the generation the
    /// stream was opened for, which later tracks may have joined.
    StreamInvalidated(Generation),
    /// Names the generation the stream was opened for.
    OutputFailed(Generation, String),
    Shutdown,
}

struct Shared {
    frames_played: AtomicU64,
    output_rate: AtomicU32,
    /// `f32` bits; read by the audio callback.
    volume: AtomicU32,
    issued: AtomicU64,
    generation: AtomicU64,
    /// Never touched by the audio callback.
    cancels: Mutex<Cancels>,
}

#[derive(Default)]
struct Cancels {
    current: Option<Cancel>,
    next: Option<(Generation, Option<Cancel>)>,
}

impl Shared {
    fn new() -> Self {
        Self {
            frames_played: AtomicU64::new(0),
            output_rate: AtomicU32::new(0),
            volume: AtomicU32::new(1.0f32.to_bits()),
            issued: AtomicU64::new(0),
            generation: AtomicU64::new(0),
            cancels: Mutex::default(),
        }
    }

    fn position(&self) -> Duration {
        let rate = self.output_rate.load(Ordering::Relaxed);
        if rate == 0 {
            return Duration::ZERO;
        }
        let frames = self.frames_played.load(Ordering::Relaxed);
        Duration::from_secs_f64(frames as f64 / f64::from(rate))
    }

    fn is_current(&self, generation: Generation) -> bool {
        self.generation.load(Ordering::Acquire) == generation
    }

    fn lock_cancels(&self) -> MutexGuard<'_, Cancels> {
        self.cancels.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn issue(&self) -> Generation {
        self.issued.fetch_add(1, Ordering::AcqRel) + 1
    }

    /// Starts a new generation and cancels the previous track and the one
    /// set to follow it, so their decoding threads do not stay blocked on a read.
    fn supersede(&self, cancel: Option<Cancel>) -> Generation {
        let (generation, previous) = {
            let mut cancels = self.lock_cancels();
            let generation = self.issue();
            self.generation.store(generation, Ordering::Release);
            let replacement = Cancels {
                current: cancel,
                next: None,
            };
            (generation, std::mem::replace(&mut *cancels, replacement))
        };
        if let Some(cancel) = previous.current {
            cancel();
        }
        if let Some((_, Some(cancel))) = previous.next {
            cancel();
        }
        generation
    }

    /// Names a track to follow the current one, canceling the one set to
    /// before, whose reads could otherwise wait on the network for nothing.
    fn set_next(&self, cancel: Option<Cancel>) -> Generation {
        let (generation, previous) = {
            let mut cancels = self.lock_cancels();
            let generation = self.issue();
            (generation, cancels.next.replace((generation, cancel)))
        };
        if let Some((_, Some(cancel))) = previous {
            cancel();
        }
        generation
    }

    fn clear_next(&self) {
        let previous = self.lock_cancels().next.take();
        if let Some((_, Some(cancel))) = previous {
            cancel();
        }
    }

    /// Makes `to`, which was set to follow `from`, the current track, unless
    /// a later request has replaced `from`. Returns whether it did.
    fn promote(&self, from: Generation, to: Generation) -> bool {
        let mut cancels = self.lock_cancels();
        if !self.is_current(from) {
            return false;
        }
        self.generation.store(to, Ordering::Release);
        cancels.current = cancels
            .next
            .take_if(|(generation, _)| *generation == to)
            .and_then(|(_, cancel)| cancel);
        true
    }

    /// Drops the cancel hook of a track that has finished, so the hook does
    /// not keep its source (often a whole downloaded file) alive.
    fn release(&self, generation: Generation) {
        let mut cancels = self.lock_cancels();
        if self.is_current(generation) {
            cancels.current.take();
        }
    }

    /// Drops the cancel hook of a next track that came too late to follow.
    fn release_next(&self, generation: Generation) {
        self.lock_cancels()
            .next
            .take_if(|(next, _)| *next == generation);
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

    /// Plays `track` when the current one ends, instead of any track set to
    /// before. Events about it carry the returned id, starting from the
    /// [`EventKind::Ended`] that names it.
    ///
    /// The track starts opening right away on a thread of its own, so waiting
    /// for its data does not hold up the current one. If the device can stay
    /// as it is, the track's first sample follows the current one's last on
    /// the same stream; otherwise the stream reopens in between. Failing to
    /// open is reported when the track's turn comes.
    ///
    /// With no current track, as right after one ended, nothing follows and
    /// the track is dropped.
    pub fn set_next(&self, mut track: Track) -> LoadId {
        let generation = self.shared.set_next(track.cancel.take());
        self.send(Command::SetNext(generation, track));
        LoadId(generation)
    }

    /// Lets nothing follow the current track. Audio of the next track that is
    /// already on its way to the device is taken back.
    pub fn clear_next(&self) {
        self.shared.clear_next();
        self.send(Command::ClearNext);
    }

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
    /// for a download may mean waiting for it to arrive, and the position
    /// then reads where it could go on from. A track that cannot seek fails
    /// with [`EventKind::Failed`].
    pub fn seek(&self, to: Duration) {
        self.send(Command::Seek(to));
    }

    /// Plays on `output` from now on. A track playing or paused moves there
    /// at once and reports [`EventKind::OutputChanged`], or
    /// [`EventKind::Failed`] if it cannot.
    pub fn set_output(&self, output: OutputSettings) {
        self.send(Command::SetOutput(output));
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
        self.shared.position()
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
        // Feeds end once a blocked read returns, so unblock those first
        self.shared.supersede(None);
        self.send(Command::Shutdown);
        Some(thread)
    }

    fn send(&self, command: Command) {
        // The thread exits only on shutdown, after which commands have no effect
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
    // `serve` has closed the last stream, so the device is free to switch
    if let Some(restore) = restore {
        restore.apply();
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "one loop on the engine's thread, whose locals every command changes"
)]
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
    let mut output = OutputSettings::default();
    let mut current: Option<Playback> = None;
    // The track to play once it has opened, before which there is no current
    let mut pending: Option<Pending> = None;
    let mut next: Option<Next> = None;
    let mut buffering = BufferingReport::default();
    let mut idle_since: Option<Instant> = None;
    loop {
        if current.is_some() || pending.is_some() {
            idle_since = None;
        } else {
            idle_since.get_or_insert_with(Instant::now);
        }
        buffering.update(
            pending
                .as_ref()
                .filter(|pending| !pending.start.paused)
                .map(|pending| pending.track.generation)
                .or_else(|| current.as_ref().and_then(Playback::waiting_generation)),
            &emit,
        );
        let playing = current.as_ref().is_some_and(|playback| !playback.paused);
        let timeout = if current.as_ref().is_some_and(Playback::has_more_to_fill) {
            Some(Duration::ZERO)
        } else if playing || pending.is_some() {
            Some(FILL_INTERVAL)
        } else {
            idle_since
                .filter(|_| restore.is_some())
                .map(|since| RESTORE_DELAY.saturating_sub(since.elapsed()))
        };
        let command = match timeout {
            Some(timeout) => match commands.recv_timeout(timeout) {
                Ok(command) => Some(command),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            },
            None => match commands.recv() {
                Ok(command) => Some(command),
                Err(_) => return,
            },
        };
        if command.is_none() && current.is_none() && pending.is_none() {
            // Nothing has played for `RESTORE_DELAY`, so no next track is coming
            if let Some(restore) = restore.take() {
                restore.apply();
            }
            continue;
        }

        // An error names the generation it ends
        let result: Result<(), (Generation, Error)> = match command {
            None => Ok(()),
            Some(Command::Load(generation, track)) => {
                // Release the device before opening it again
                current = None;
                pending = None;
                next = None;
                buffering = BufferingReport::default();
                // A newer request already replaced this one; dropping the
                // track releases its source.
                if shared.is_current(generation) {
                    pending = Some(Pending::load(generation, track));
                }
                Ok(())
            }
            Some(Command::SetNext(generation, track)) => {
                let abandoned = abandon_next(&mut current, shared, &emit);
                if current.is_some() || pending.is_some() {
                    next = Some(Next::open(generation, track));
                } else {
                    shared.release_next(generation);
                }
                abandoned
            }
            Some(Command::ClearNext) => {
                next = None;
                abandon_next(&mut current, shared, &emit)
            }
            Some(Command::Pause) => {
                if let Some(pending) = &mut pending {
                    pending.start.paused = true;
                }
                current.as_mut().map_or(Ok(()), |playback| {
                    playback
                        .pause()
                        .map_err(|error| (playback.generation, error))
                })
            }
            Some(Command::Resume) => {
                if let Some(pending) = &mut pending {
                    pending.start.paused = false;
                }
                current.as_mut().map_or(Ok(()), |playback| {
                    playback
                        .resume()
                        .map_err(|error| (playback.generation, error))
                })
            }
            Some(Command::Seek(to)) => {
                if let Some(pending) = &mut pending {
                    pending.seek(to);
                }
                return_to_sounding(&mut current, &mut next, shared, &emit);
                current.as_mut().map_or(Ok(()), |playback| {
                    playback
                        .seek(to, shared)
                        .map_err(|error| (playback.generation, error))
                })
            }
            Some(Command::SetOutput(settings)) if settings == output => Ok(()),
            // Moving would interrupt the sound for a change that cannot be heard
            Some(Command::SetOutput(settings))
                if settings.has_same_device_and_rate(&output)
                    && !current.as_ref().is_some_and(Playback::may_resample) =>
            {
                output = settings;
                Ok(())
            }
            Some(Command::SetOutput(settings)) => {
                if settings.dop().is_some()
                    && output.dop().is_none()
                    && let Some(pending) = &mut pending
                {
                    pending.start.fades_in = true;
                }
                output = settings;
                return_to_sounding(&mut current, &mut next, shared, &emit);
                current.take().map_or(Ok(()), |playback| {
                    let moved = playback.move_to(&output, shared, command_sender, restore)?;
                    current = moved.inspect(|playback| {
                        emit(
                            playback.generation,
                            EventKind::OutputChanged(playback.format.clone()),
                        );
                    });
                    Ok(())
                })
            }
            Some(Command::StreamInvalidated(generation)) => {
                // A late notice about an earlier stream changes nothing here
                if current
                    .as_ref()
                    .is_some_and(|playback| playback.opened_for == generation)
                {
                    return_to_sounding(&mut current, &mut next, shared, &emit);
                }
                current
                    .take_if(|playback| playback.opened_for == generation)
                    .map_or(Ok(()), |playback| {
                        let settings = keep_device_rate(&output, &playback, restore);
                        let moved = playback.move_to(&settings, shared, command_sender, restore)?;
                        current = moved.inspect(|playback| {
                            emit(
                                playback.generation,
                                EventKind::OutputChanged(playback.format.clone()),
                            );
                        });
                        Ok(())
                    })
            }
            Some(Command::Stop) => {
                current = None;
                pending = None;
                next = None;
                buffering = BufferingReport::default();
                Ok(())
            }
            Some(Command::OutputFailed(generation, message)) => current
                .as_ref()
                .filter(|playback| playback.opened_for == generation)
                .map_or(Ok(()), |playback| {
                    Err((playback.generation, Error::Output(message)))
                }),
            Some(Command::Shutdown) => return,
        };
        if let Err((generation, error)) = result {
            if let Some(playback) = current.take() {
                shared.release(playback.generation);
            }
            // A superseded track was canceled on purpose, so stay silent
            if shared.is_current(generation) {
                emit(generation, EventKind::Failed(error));
            }
            continue;
        }

        report_crossing(&mut current, shared, &emit);

        if let Some(waiting) = pending.take() {
            match waiting.poll() {
                Err(waiting) => pending = Some(waiting),
                Ok((generation, opened, start)) if shared.is_current(generation) => {
                    let opened = opened.and_then(|feed| {
                        Playback::begin(
                            feed,
                            start,
                            generation,
                            &output,
                            shared,
                            command_sender,
                            restore,
                        )
                    });
                    match adopt(opened, generation, shared, &emit) {
                        Ok(playback) => current = playback,
                        Err((failed, error)) => emit(failed, EventKind::Failed(error)),
                    }
                }
                Ok(_) => {}
            }
        }

        if let Some(playback) = current.as_mut().filter(|playback| !playback.paused) {
            let generation = playback.generation;
            next = next.and_then(|next| playback.join(next, &output, shared));
            let fill = playback.fill(shared);
            let was_fading = playback.converter.is_fading();
            if matches!(fill, Ok(Fill::Playing)) {
                continue;
            }
            current = None;
            // A superseded track was canceled on purpose, so stay silent
            if !shared.is_current(generation) {
                continue;
            }
            match fill {
                Err(error) => {
                    shared.release(generation);
                    emit(generation, EventKind::Failed(error));
                }
                Ok(_) => match next.take() {
                    None => {
                        shared.release(generation);
                        emit(generation, EventKind::Ended { next: None });
                    }
                    Some(following) => {
                        let started = following.generation;
                        if !shared.promote(generation, started) {
                            continue;
                        }
                        let next = Some(LoadId(started));
                        emit(generation, EventKind::Ended { next });
                        pending = Some(Pending {
                            track: following,
                            start: Start {
                                fades_in: was_fading,
                                ..Start::BEGINNING
                            },
                        });
                    }
                },
            }
        }
    }
}

/// Reports when a track has been silent for [`BUFFERING_NOTICE`] waiting for
/// its data, and when that is over.
#[derive(Default)]
struct BufferingReport {
    waiting_since: Option<Instant>,
    reported: Option<Generation>,
}

impl BufferingReport {
    fn update(&mut self, waiting: Option<Generation>, emit: &impl Fn(Generation, EventKind)) {
        let Some(generation) = waiting else {
            self.waiting_since = None;
            if let Some(reported) = self.reported.take() {
                emit(reported, EventKind::Buffering(false));
            }
            return;
        };
        // Another track took over, so the wait told about is no longer anyone's
        if self.reported.is_some_and(|reported| reported != generation) {
            *self = Self::default();
        }
        let since = *self.waiting_since.get_or_insert_with(Instant::now);
        if self.reported.is_none() && since.elapsed() >= BUFFERING_NOTICE {
            self.reported = Some(generation);
            emit(generation, EventKind::Buffering(true));
        }
    }
}

/// Takes in the outcome of opening the current track, reporting that it
/// started. A superseded track was canceled on purpose and stays silent. An
/// error names the generation it ends.
fn adopt(
    opened: Result<Playback, Error>,
    generation: Generation,
    shared: &Shared,
    emit: &impl Fn(Generation, EventKind),
) -> Result<Option<Playback>, (Generation, Error)> {
    match opened {
        Ok(playback) if shared.is_current(generation) => {
            emit(generation, EventKind::Started(playback.format.clone()));
            Ok(Some(playback))
        }
        Ok(_) => Ok(None),
        Err(_) if !shared.is_current(generation) => Ok(None),
        Err(error) => {
            shared.release(generation);
            Err((generation, error))
        }
    }
}

/// Reports that the device has gone from one track to the one that joined it
/// on the stream.
fn report_crossing(
    current: &mut Option<Playback>,
    shared: &Shared,
    emit: &impl Fn(Generation, EventKind),
) {
    let Some(playback) = current.as_mut() else {
        return;
    };
    let Some(joined) = playback.take_crossed() else {
        return;
    };
    let ended = std::mem::replace(&mut playback.generation, joined.generation);
    // How long the last track had to wait is no reason for this one to
    playback.hold = FIRST_HOLD;
    if shared.promote(ended, joined.generation) {
        let next = Some(LoadId(joined.generation));
        emit(ended, EventKind::Ended { next });
        // One that has already failed goes on to report that instead
        if playback.failure.is_none() {
            emit(
                joined.generation,
                EventKind::Started(playback.format.clone()),
            );
        }
    }
}

/// Before a change to where the sounding track is: makes it the one being
/// written again, and the track that joined it the next one once more.
fn return_to_sounding(
    current: &mut Option<Playback>,
    next: &mut Option<Next>,
    shared: &Shared,
    emit: &impl Fn(Generation, EventKind),
) {
    if let Some(taken) = current.as_mut().and_then(Playback::take_back) {
        *next = Some(taken.rewound());
    }
    report_crossing(current, shared, emit);
}

/// Before the next track changes: drops the track that joined the sounding
/// one, which goes on from where the device is. An error names the generation
/// it ends.
fn abandon_next(
    current: &mut Option<Playback>,
    shared: &Shared,
    emit: &impl Fn(Generation, EventKind),
) -> Result<(), (Generation, Error)> {
    // Read first: as long as the take-back works, this is still the position
    // in the sounding track.
    let position = shared.position();
    let taken_back = current.as_mut().and_then(Playback::take_back).is_some();
    let result = match current.as_mut() {
        Some(playback) if taken_back => playback
            .seek(position, shared)
            .map_err(|error| (playback.generation, error)),
        _ => Ok(()),
    };
    report_crossing(current, shared, emit);
    result
}

/// Whether a stream error ends playback. cpal reports some conditions on the
/// error callback while the stream keeps running: an overload (`Xrun`) on
/// every glitch, a reroute of the default device, or a denied real-time
/// priority. Treating those as failures would stop a track over a click.
fn is_fatal(kind: cpal::ErrorKind) -> bool {
    use cpal::ErrorKind::*;
    !matches!(kind, Xrun | DeviceChanged | RealtimeDenied)
}

/// A stream on the system's default device follows it by itself. What holds a
/// device does not, and would keep other apps off the old one while the sound
/// comes from the new one, in reach of them.
fn must_reopen(kind: cpal::ErrorKind, holds_device: bool) -> bool {
    match kind {
        cpal::ErrorKind::StreamInvalidated => true,
        cpal::ErrorKind::DeviceChanged => holds_device,
        _ => false,
    }
}

enum Fill {
    Playing,
    /// The device has played every sample of the track.
    Drained,
}

struct Next {
    generation: Generation,
    state: NextState,
}

enum NextState {
    /// On a thread of its own, as that may wait on the network.
    Opening(Opening),
    Ready(Feed),
    /// Reported when its turn comes.
    Failed(Error),
}

impl Next {
    fn open(generation: Generation, track: Track) -> Self {
        let state = match Opening::start(track, FIRST_HOLD) {
            Ok(opening) => NextState::Opening(opening),
            Err(error) => NextState::Failed(error),
        };
        Self { generation, state }
    }

    fn polled(self) -> Self {
        let state = match self.state {
            NextState::Opening(opening) => match opening.poll() {
                Polled::Waiting(opening) => NextState::Opening(opening),
                Polled::Opened(feed) => NextState::Ready(feed),
                Polled::Failed(error) => NextState::Failed(error),
            },
            state => state,
        };
        Self { state, ..self }
    }

    /// Back at its first sample, after decoding some.
    fn rewound(self) -> Self {
        if let NextState::Ready(feed) = &self.state {
            feed.seek(Duration::ZERO);
        }
        self
    }
}

/// The current track while it opens. Everything asked of it meanwhile goes in
/// [`Self::start`].
struct Pending {
    track: Next,
    start: Start,
}

impl Pending {
    fn load(generation: Generation, track: Track) -> Self {
        let position = track.start_at;
        let paused = track.start_paused;
        let fades_in = track.fades_in;
        let mut pending = Self {
            track: Next::open(generation, track),
            start: Start {
                paused,
                fades_in,
                ..Start::BEGINNING
            },
        };
        if !position.is_zero() {
            pending.seek(position);
        }
        pending
    }

    fn seek(&mut self, to: Duration) {
        self.start.position = to;
        match &self.track.state {
            NextState::Opening(opening) => opening.seek(to),
            NextState::Ready(feed) => feed.seek(to),
            NextState::Failed(_) => {}
        }
    }

    fn poll(self) -> Result<(Generation, Result<Feed, Error>, Start), Self> {
        let Self { track, start } = self;
        let track = track.polled();
        let generation = track.generation;
        match track.state {
            NextState::Opening(_) => Err(Self { track, start }),
            NextState::Ready(feed) => Ok((generation, Ok(feed), start)),
            NextState::Failed(error) => Ok((generation, Err(error), start)),
        }
    }
}

/// The track still sounding while a next one is written behind it, kept to go
/// back to until the device has reached the next one.
struct Joined {
    /// Of the next track; the playback still has the sounding one's.
    generation: Generation,
    feed: Feed,
    /// `None` when the next track goes through the same one.
    converter: Option<Converter>,
    format: PlaybackFormat,
}

/// What the engine needs to switch a device back to the sample rate it had
/// before the engine first changed it.
///
/// On macOS the rate a stream asks for becomes the device's rate for every
/// app, and it stays after the stream closes. Other platforms either convert
/// per stream or release the rate with the stream, so nothing is recorded
/// there.
///
/// The rate comes back, and the physical format if holding the device replaced
/// it. cpal may also change that format on its own, such as 24-bit integer to
/// 32-bit float, and has no way to restore it.
struct RateRecord<D> {
    device: D,
    /// The device's configuration before the first switch.
    original: cpal::SupportedStreamConfig,
    /// The rate the engine last set. A different rate means someone else has
    /// changed it since, and theirs is kept.
    switched_to: u32,
    format: Option<OriginalFormat>,
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
                    format: None,
                });
            }
            _ => {}
        }
    }

    /// Notes that holding `device` replaced its physical format, keeping the
    /// first one replaced since nothing played.
    fn note_format(
        slot: &mut Option<Self>,
        device: D,
        current: cpal::SupportedStreamConfig,
        rate: u32,
        format: OriginalFormat,
    ) {
        match slot {
            Some(record) if record.device == device => {
                record.format.get_or_insert(format);
            }
            _ => {
                *slot = Some(Self {
                    device,
                    original: current,
                    switched_to: rate,
                    format: Some(format),
                });
            }
        }
    }

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
        if self.needs_restore(current.sample_rate()) {
            self.restore_rate(&device);
        }
        if let Some(format) = self.format {
            format.restore();
        }
    }

    fn restore_rate(&self, device: &cpal::Device) {
        // cpal sets a device to a stream's rate when it builds the stream, so
        // building one at the old rate switches the device back. The stream
        // never starts and closes right away.
        let stream = device.build_output_stream_raw(
            self.original.config(),
            self.original.sample_format(),
            |_: &mut cpal::Data, _: &cpal::OutputCallbackInfo| {},
            |_| {},
            Some(RATE_SWITCH_TIMEOUT),
        );
        drop(stream);
    }
}

fn find_device(output: &OutputSettings) -> Result<cpal::Device, Error> {
    let host = cpal::default_host();
    match output.device() {
        None => host.default_output_device().ok_or(Error::NoOutputDevice),
        // An id that cannot be read may come from another platform's settings
        Some(id) => id
            .parse()
            .ok()
            .and_then(|id| host.device_by_id(&id))
            .ok_or(Error::OutputDeviceUnavailable),
    }
}

/// `output` at the rate the device runs at now, for a stream whose device has
/// changed under it. Whoever changed the rate keeps it: asking for the old one
/// again could fight them, and it stays when playback ends, so nothing is left
/// to restore. The next track follows `output` again.
///
/// A held device has no one else to have changed it, and on Windows what it
/// reports is the rate of a shared mode it is not in.
fn keep_device_rate(
    output: &OutputSettings,
    playback: &Playback,
    restore: &mut Option<RateRestore>,
) -> OutputSettings {
    let stream_rate = playback.format.output_rate;
    let device_rate = if output.is_exclusive() {
        stream_rate
    } else {
        find_device(output)
            .and_then(|device| Ok(device.default_output_config()?.sample_rate()))
            .unwrap_or(stream_rate)
    };
    if device_rate != stream_rate {
        *restore = None;
    }
    OutputSettings::new(output.device().map(str::to_owned), Some(device_rate))
        .with_resampling(output.resampling())
        .with_dop(output.dop().copied())
        .with_dop_fade_in(output.dop_fades_in())
        .with_exclusive(output.is_exclusive())
}

#[derive(Clone, Copy)]
struct Start {
    position: Duration,
    paused: bool,
    fades_in: bool,
}

impl Start {
    const BEGINNING: Self = Self {
        position: Duration::ZERO,
        paused: false,
        fades_in: false,
    };
}

/// A device stream and the track it plays. Lives on the engine thread only;
/// `cpal::Stream` is not `Send` on every platform. A new stream is opened
/// whenever the device has to follow a file's sample rate; a track it need not
/// change for joins the same stream, with no gap.
struct Playback {
    /// Of the track being written, which may be behind the one sounding; see
    /// [`Self::joined`].
    feed: Feed,
    converter: Converter,
    producer: rtrb::Producer<f32>,
    capacity: usize,
    /// Converted samples not yet in the ring buffer.
    pending: Vec<f32>,
    pending_ix: usize,
    /// The feed has no more samples; the converter may still hold some.
    decoded_all: bool,
    /// The converter has given up its last samples too.
    finished: bool,
    paused: bool,
    /// Nothing goes to the device, which plays silence and whose position
    /// stands still, until the feed has [`Self::hold`] ready.
    holding: bool,
    hold: Duration,
    /// Of the track being written.
    format: PlaybackFormat,
    /// Of the track sounding.
    generation: Generation,
    /// The generation the stream was opened for, by which its errors name it.
    opened_for: Generation,
    /// Set while paused on DoP, whose stream keeps running with DoP silence so
    /// that the DAC stays locked on it.
    dop_paused: Option<Arc<AtomicBool>>,
    /// The device's configurations, to tell whether a next track fits.
    ranges: Vec<ConfigRange>,
    joined: Option<Joined>,
    /// Why the track being written stopped, reported once it is sounding.
    failure: Option<Error>,
    /// The number of the first sample of the track that joined, until the
    /// audio callback takes samples up to it and sets this to zero. `0` means
    /// the device has no track to reach: none is on its way, or it is there.
    next_at: Arc<AtomicU64>,
    /// Set by the audio callback while samples it handed to the device have
    /// not all been played.
    sounding: Arc<AtomicBool>,
    /// When the ring buffer ran empty after the last sample was decoded.
    emptied_at: Option<Instant>,
    /// Samples ever written to the ring buffer.
    written: u64,
    out_of_time: bool,
    /// The audio callback drops samples numbered below this instead of
    /// playing them: what was buffered before a seek.
    flush_until: Arc<AtomicU64>,
    stream: Stream,
    /// Dropped after the stream, which is playing on what it holds.
    exclusive: Option<Exclusive>,
}

impl Playback {
    /// A joined track can have a resampler the sounding one does not. DoP has
    /// none, whichever track it is.
    fn may_resample(&self) -> bool {
        self.format.dop.is_none() && (self.format.is_resampled() || self.joined.is_some())
    }

    /// Goes on from the same position on `settings`, paused if it was.
    fn reopen(
        self,
        settings: &OutputSettings,
        shared: &Arc<Shared>,
        commands: &mpsc::Sender<Command>,
        restore: &mut Option<RateRestore>,
    ) -> Result<Self, Error> {
        let start = Start {
            position: shared.position(),
            paused: self.paused,
            fades_in: settings.dop().is_some(),
        };
        let Self {
            feed,
            generation,
            stream,
            exclusive,
            ..
        } = self;
        // Release the device before opening it again
        drop(stream);
        drop(exclusive);
        feed.seek(start.position);
        Self::begin(feed, start, generation, settings, shared, commands, restore)
    }

    /// [`Self::reopen`], leaving out a track superseded meanwhile. An error
    /// names the generation it ends.
    fn move_to(
        self,
        settings: &OutputSettings,
        shared: &Arc<Shared>,
        commands: &mpsc::Sender<Command>,
        restore: &mut Option<RateRestore>,
    ) -> Result<Option<Self>, (Generation, Error)> {
        let generation = self.generation;
        match self.reopen(settings, shared, commands, restore) {
            Ok(playback) if shared.is_current(generation) => Ok(Some(playback)),
            Ok(_) => Ok(None),
            Err(_) if !shared.is_current(generation) => Ok(None),
            Err(error) => {
                shared.release(generation);
                Err((generation, error))
            }
        }
    }

    /// Opens a stream for `feed`, which is at `start`, and starts it.
    fn begin(
        feed: Feed,
        start: Start,
        generation: Generation,
        settings: &OutputSettings,
        shared: &Arc<Shared>,
        commands: &mpsc::Sender<Command>,
        restore: &mut Option<RateRestore>,
    ) -> Result<Self, Error> {
        let mut playback = Self::open(
            feed,
            generation,
            settings,
            start.position,
            shared,
            commands,
            restore,
        )?;
        if start.fades_in && settings.dop_fades_in() {
            playback.converter.fade_in(DOP_FADE_IN);
        }
        // Whatever is ready by now, so the first callbacks need not be silent.
        // Nothing takes from the ring buffer yet, so this ends when it is full.
        playback.fill(shared)?;
        while playback.has_more_to_fill() {
            playback.fill(shared)?;
        }
        if start.paused {
            playback.pause()?;
        } else {
            playback.stream.play()?;
        }
        Ok(playback)
    }

    /// Opens a stream for `feed`, which has yet to be filled and started.
    /// `position` is where the feed is in the track.
    fn open(
        feed: Feed,
        generation: Generation,
        settings: &OutputSettings,
        position: Duration,
        shared: &Arc<Shared>,
        commands: &mpsc::Sender<Command>,
        restore: &mut Option<RateRestore>,
    ) -> Result<Self, Error> {
        let source = feed.spec();

        let device = find_device(settings)?;
        let device_id = cfg!(target_os = "macos")
            .then(|| device.id().ok())
            .flatten();
        if let Some(left) = RateRestore::take_if_other(restore, device_id.as_ref()) {
            left.apply();
        }
        let ranges = output::ranges_for(&device, settings)?;
        // Held until the stream exists, which is when cpal switches the rate
        let setup = DEVICE_SETUP.lock().unwrap_or_else(PoisonError::into_inner);
        let device_config = device.default_output_config()?;
        let device_rate = device_config.sample_rate();
        let dop = settings.dop().map(dsd::Settings::rate);
        let output =
            output::plan_for(settings, &ranges, device_rate, source).ok_or(if dop.is_some() {
                Error::UnsupportedDopRate
            } else {
                Error::UnsupportedOutputFormat
            })?;
        if let Some(device_id) = &device_id {
            RateRestore::note(restore, device_id.clone(), device_config, output.rate);
        }
        let exclusive = if dop.is_some() {
            Some(Exclusive::for_dop(&device, output.rate)?)
        } else if settings.is_exclusive() {
            Some(Exclusive::for_pcm(&device, output.rate)?)
        } else {
            None
        };
        if let (Some(device_id), Some(format)) = (
            device_id,
            exclusive.as_ref().and_then(Exclusive::replaced_format),
        ) {
            RateRestore::note_format(restore, device_id, device_config, output.rate, format);
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
        let next_at = Arc::new(AtomicU64::new(0));

        let frames = (position.as_secs_f64() * f64::from(output.rate)) as u64;
        shared.frames_played.store(frames, Ordering::Relaxed);
        shared.output_rate.store(output.rate, Ordering::Relaxed);
        let sounding = Arc::new(AtomicBool::new(false));
        let flush_until = Arc::new(AtomicU64::new(0));
        let reader = RingReader::new(
            consumer,
            channels,
            output.rate,
            Arc::clone(shared),
            Arc::clone(&sounding),
            Arc::clone(&flush_until),
            Arc::clone(&next_at),
        );
        let holds_device = exclusive.is_some();
        let on_error = {
            let commands = commands.clone();
            move |error: cpal::Error| {
                if must_reopen(error.kind(), holds_device) {
                    commands.send(Command::StreamInvalidated(generation)).ok();
                } else if is_fatal(error.kind()) {
                    commands
                        .send(Command::OutputFailed(generation, error.to_string()))
                        .ok();
                }
            }
        };
        let dop_paused = dop.map(|_| Arc::new(AtomicBool::new(false)));
        let stream = match &dop_paused {
            None => Stream::open_pcm(
                &device,
                config,
                settings,
                output_callback(reader, Arc::clone(shared)),
                on_error,
                RATE_SWITCH_TIMEOUT,
            )?,
            // Integers, as asking for them is what gets cpal to set an
            // integer-only device to more than 16 bits
            Some(paused) => Stream::Shared(device.build_output_stream(
                config,
                dop_callback(reader, output.rate, Arc::clone(paused)),
                on_error,
                Some(RATE_SWITCH_TIMEOUT),
            )?),
        };
        // PCM takes whatever format the device has, which the stream converts to
        if let Some(exclusive) = exclusive.as_ref().filter(|_| dop.is_some()) {
            exclusive.confirm(output.rate)?;
        }
        drop(setup);

        Ok(Self {
            converter: Converter::for_settings(source, output, settings)?,
            format: PlaybackFormat {
                device: device_name(&device),
                source_rate: source.rate,
                source_channels: source.channels,
                output_rate: output.rate,
                output_channels: output.channels,
                dop,
                duration: feed.duration(),
            },
            feed,
            producer,
            capacity,
            pending: Vec::new(),
            pending_ix: 0,
            decoded_all: false,
            finished: false,
            paused: false,
            holding: true,
            hold: FIRST_HOLD,
            generation,
            opened_for: generation,
            dop_paused,
            ranges,
            joined: None,
            failure: None,
            next_at,
            sounding,
            emptied_at: None,
            written: 0,
            out_of_time: false,
            flush_until,
            stream,
            exclusive,
        })
    }

    fn pause(&mut self) -> Result<(), Error> {
        match &self.dop_paused {
            None => self.stream.pause()?,
            Some(dop_paused) => {
                dop_paused.store(true, Ordering::Release);
                self.stream.play()?;
            }
        }
        self.paused = true;
        Ok(())
    }

    fn resume(&mut self) -> Result<(), Error> {
        if let Some(dop_paused) = &self.dop_paused {
            dop_paused.store(false, Ordering::Release);
        }
        self.stream.play()?;
        self.paused = false;
        Ok(())
    }

    /// The track the device has to wait for, as it plays silence for want of
    /// data.
    fn waiting_generation(&self) -> Option<Generation> {
        (self.holding && !self.paused).then_some(self.generation)
    }

    fn seek(&mut self, to: Duration, shared: &Shared) -> Result<(), Error> {
        // Silence what is buffered at once, as the seek may wait for data
        self.flush_until.store(self.written, Ordering::Release);
        shared
            .frames_played
            .store(self.frames_in(to), Ordering::Relaxed);
        self.feed.seek(to);
        self.converter.reset()?;
        self.pending.clear();
        self.pending_ix = 0;
        self.finished = false;
        self.decoded_all = false;
        self.emptied_at = None;
        self.holding = true;
        // Whoever seeks is waiting to hear it, so not as long as it may have
        // taken to play on before.
        self.hold = FIRST_HOLD;
        Ok(())
    }

    fn frames_in(&self, time: Duration) -> u64 {
        (time.as_secs_f64() * f64::from(self.format.output_rate)) as u64
    }

    /// Writes `next` behind the current track, if `next` plays on this stream
    /// as it is and that track has no samples left to decode. Otherwise gives
    /// `next` back.
    ///
    /// A next track with the same source format goes through the same
    /// converter. A resampler flushed at the end of a track dents the audio
    /// on either side of the joint, so this is tried before it is flushed.
    fn join(&mut self, next: Next, settings: &OutputSettings, shared: &Shared) -> Option<Next> {
        let next = next.polled();
        let all_written = self.finished && self.pending_ix == self.pending.len();
        let ready = all_written || (self.decoded_all && !self.finished);
        if !ready || self.joined.is_some() || self.failure.is_some() {
            return Some(next);
        }
        let generation = next.generation;
        let output = Spec {
            rate: self.format.output_rate,
            channels: self.format.output_channels,
        };
        let feed = match next.state {
            NextState::Ready(feed)
                if output::plan_for(settings, &self.ranges, output.rate, feed.spec())
                    == Some(output)
                    // Otherwise it would sound what little has arrived, and
                    // then go silent. It goes on at its turn instead.
                    && feed.has_ahead(FIRST_HOLD) =>
            {
                feed
            }
            state => return Some(Next { generation, state }),
        };
        let source = feed.spec();
        let converter = if self.finished {
            match self.converter.following(source, settings) {
                Ok(converter) => Some(converter),
                Err(error) => {
                    let state = NextState::Failed(error);
                    return Some(Next { generation, state });
                }
            }
        } else if source.rate == self.format.source_rate
            && source.channels == self.format.source_channels
        {
            None
        } else {
            let state = NextState::Ready(feed);
            return Some(Next { generation, state });
        };
        let next_at = self.written
            + self.converter.undelivered_frames() * u64::from(self.format.output_channels);
        let format = PlaybackFormat {
            device: self.format.device.clone(),
            source_rate: source.rate,
            source_channels: source.channels,
            output_rate: output.rate,
            output_channels: output.channels,
            dop: self.format.dop,
            duration: feed.duration(),
        };
        self.joined = Some(Joined {
            generation,
            feed: std::mem::replace(&mut self.feed, feed),
            converter: converter.map(|converter| std::mem::replace(&mut self.converter, converter)),
            format: std::mem::replace(&mut self.format, format),
        });
        self.decoded_all = false;
        self.finished = false;
        self.emptied_at = None;
        // After no samples at all, the device is at the next track already,
        // and the callback sees no crossing to restart the position at.
        if next_at == 0 {
            shared.frames_played.store(0, Ordering::Relaxed);
        }
        self.next_at.store(next_at, Ordering::Release);
        None
    }

    /// The track that joined, once the device has reached it.
    fn take_crossed(&mut self) -> Option<Joined> {
        self.joined
            .take_if(|_| self.next_at.load(Ordering::Acquire) == 0)
    }

    /// Goes back to writing the sounding track, and returns the track that
    /// joined it, unless the device has reached that one. Follow with a seek:
    /// the sounding track's feed is at its end, and its next samples are
    /// the ones to play after what the ring buffer holds is flushed.
    fn take_back(&mut self) -> Option<Next> {
        self.joined.as_ref()?;
        // Whichever of this and the audio callback sets it to zero first
        // decides whether the device reaches the next track.
        if self.next_at.swap(0, Ordering::AcqRel) == 0 {
            return None;
        }
        let joined = self.joined.take()?;
        let feed = std::mem::replace(&mut self.feed, joined.feed);
        if let Some(converter) = joined.converter {
            self.converter = converter;
        }
        self.format = joined.format;
        let state = match self.failure.take() {
            Some(error) => NextState::Failed(error),
            None => NextState::Ready(feed),
        };
        Some(Next {
            generation: joined.generation,
            state,
        })
    }

    /// Whether the last fill stopped for time rather than for want of room or
    /// data.
    fn has_more_to_fill(&self) -> bool {
        self.out_of_time && !self.paused
    }

    fn fill(&mut self, shared: &Shared) -> Result<Fill, Error> {
        self.out_of_time = false;
        // A next track's failure is not the sounding track's, whose last
        // samples may still be on their way here.
        if self.joined.is_some() && self.failure.is_some() {
            self.write_pending();
            return Ok(Fill::Playing);
        }
        if let Some(error) = self.failure.take() {
            return Err(error);
        }
        if self.holding && !self.feed.has_ahead(self.hold) {
            return Ok(Fill::Playing);
        }
        self.holding = false;
        let filled = match self.write_ahead(shared) {
            Err(error) if self.joined.is_some() => {
                self.give_up_on_joined(error);
                Ok(Fill::Playing)
            }
            filled => filled,
        };
        if self.has_run_out() {
            self.holding = true;
            self.hold = (self.hold * 2).min(LONGEST_HOLD);
        }
        filled
    }

    /// Whether the device has taken all there is of a track with more to
    /// come, which it should wait for.
    fn has_run_out(&self) -> bool {
        self.joined.is_none()
            && !self.finished
            && !self.decoded_all
            && self.pending_ix == self.pending.len()
            && self.producer.slots() == self.capacity
    }

    /// Stops writing the track that joined, which `error` ended, and makes
    /// sure the device can still reach it, to report that at its turn.
    ///
    /// The converter they may share holds the end of the sounding track until
    /// more input pushes it out, so it gives that up now. If it cannot, the
    /// sounding track ends with what the ring buffer got.
    fn give_up_on_joined(&mut self, error: Error) {
        self.failure = Some(error);
        self.converter.finish(&mut self.pending).ok();
        let end = self.written + (self.pending.len() - self.pending_ix) as u64;
        self.next_at.fetch_min(end, Ordering::AcqRel);
        self.write_pending();
    }

    /// Writes as much of the converted samples as the ring buffer has room
    /// for. Returns whether that was all of them.
    fn write_pending(&mut self) -> bool {
        while self.pending_ix < self.pending.len() {
            let free = self.producer.slots();
            if free == 0 {
                return false;
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
        }
        true
    }

    fn write_ahead(&mut self, shared: &Shared) -> Result<Fill, Error> {
        let started = Instant::now();
        loop {
            if !self.write_pending() {
                return Ok(Fill::Playing);
            }
            if self.finished {
                // Whatever joined counts as playing until the device reaches it
                if self.producer.slots() < self.capacity || self.joined.is_some() {
                    return Ok(Fill::Playing);
                }
                // The device has taken every sample but may still be playing
                // the last ones; closing the stream now would cut them off.
                let emptied_at = *self.emptied_at.get_or_insert_with(Instant::now);
                let played =
                    !self.sounding.load(Ordering::Acquire) || emptied_at.elapsed() >= TAIL_TIMEOUT;
                return Ok(if played { Fill::Drained } else { Fill::Playing });
            }
            // Encoding can take longer than the sound, and commands must get
            // through all the same
            if started.elapsed() >= FILL_INTERVAL {
                self.out_of_time = true;
                return Ok(Fill::Playing);
            }
            self.pending.clear();
            self.pending_ix = 0;
            if self.decoded_all {
                self.converter.finish(&mut self.pending)?;
                self.finished = true;
                continue;
            }
            match self.feed.take() {
                None => return Ok(Fill::Playing),
                Some(Item::Samples(samples)) => {
                    self.converter.process(&samples, &mut self.pending)?;
                }
                // A track that joined rewound to its start, while the device
                // is still in the sounding one.
                Some(Item::Seeked(Some(_))) if self.joined.is_some() => {}
                Some(Item::Seeked(Some(resumed))) => {
                    shared
                        .frames_played
                        .store(self.frames_in(resumed), Ordering::Relaxed);
                }
                // Past the end, the track ends once the device has played out
                Some(Item::Seeked(None)) => self.finished = true,
                Some(Item::End) => {
                    // Leaves a next track its chance to join before the
                    // converter is flushed.
                    self.decoded_all = true;
                    return Ok(Fill::Playing);
                }
                Some(Item::Failed(error)) => return Err(error),
            }
        }
    }
}

/// The side of the ring buffer in the real-time callback. No allocation,
/// locking, or blocking.
///
/// `sounding` is true from the moment samples go to the device until a later
/// callback starts after the device's predicted time for playing the last of
/// them. Samples numbered below `flush_until` are dropped unplayed. Reaching
/// `next_at` restarts the position counter, as that sample starts a track.
struct RingReader {
    consumer: rtrb::Consumer<f32>,
    channels: usize,
    rate: u32,
    shared: Arc<Shared>,
    sounding: Arc<AtomicBool>,
    flush_until: Arc<AtomicU64>,
    next_at: Arc<AtomicU64>,
    /// When the device will have played the last sample written so far
    played_by: Option<cpal::StreamInstant>,
    /// Samples taken out of the ring buffer, played or dropped
    read: u64,
}

impl RingReader {
    fn new(
        consumer: rtrb::Consumer<f32>,
        channels: usize,
        rate: u32,
        shared: Arc<Shared>,
        sounding: Arc<AtomicBool>,
        flush_until: Arc<AtomicU64>,
        next_at: Arc<AtomicU64>,
    ) -> Self {
        Self {
            consumer,
            channels,
            rate,
            shared,
            sounding,
            flush_until,
            next_at,
            played_by: None,
            read: 0,
        }
    }

    /// Takes whole frames of up to `room` samples, handing them to `copy` in
    /// two parts as the ring buffer holds them, and returns how many.
    fn take(
        &mut self,
        room: usize,
        info: &cpal::OutputCallbackInfo,
        copy: impl FnOnce(&[f32], &[f32]),
    ) -> usize {
        let stale = self
            .flush_until
            .load(Ordering::Acquire)
            .saturating_sub(self.read);
        if stale > 0 {
            let len = usize::try_from(stale)
                .unwrap_or(usize::MAX)
                .min(self.consumer.slots());
            if let Ok(chunk) = self.consumer.read_chunk(len) {
                chunk.commit_all();
                self.read += len as u64;
            }
        }
        let timestamp = info.timestamp();
        let available = self.consumer.slots().min(room);
        // Whole frames only, so an underrun never shifts the channel order
        let len = available - available % self.channels;
        let mut written = 0;
        if let Ok(chunk) = self.consumer.read_chunk(len) {
            let (first, second) = chunk.as_slices();
            copy(first, second);
            written = first.len() + second.len();
            self.read += written as u64;
            if written > 0 {
                let frames = (written / self.channels) as f64;
                self.played_by = timestamp
                    .playback
                    .checked_add(Duration::from_secs_f64(frames / f64::from(self.rate)));
                // Before the commit: whoever sees the ring buffer empty must
                // also see these samples as still sounding.
                self.sounding.store(true, Ordering::Release);
            }
            chunk.commit_all();
        }
        if written == 0
            && self
                .played_by
                .is_some_and(|played_by| timestamp.callback >= played_by)
        {
            self.played_by = None;
            self.sounding.store(false, Ordering::Release);
        }
        written
    }

    /// Moves the position on by the `written` samples just handed over.
    fn count(&mut self, written: usize) {
        let next_starts = self.next_at.load(Ordering::Acquire);
        let crossed = next_starts != 0
            && self.read >= next_starts
            && self
                .next_at
                .compare_exchange(next_starts, 0, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok();
        if crossed {
            let frames = (self.read - next_starts) / self.channels as u64;
            self.shared.frames_played.store(frames, Ordering::Relaxed);
        } else {
            self.shared
                .frames_played
                .fetch_add((written / self.channels) as u64, Ordering::Relaxed);
        }
    }
}

/// The real-time callback for PCM: copies frames out of the ring buffer and
/// fills any shortfall with silence.
fn output_callback(
    mut reader: RingReader,
    shared: Arc<Shared>,
) -> impl FnMut(&mut [f32], &cpal::OutputCallbackInfo) + Send + 'static {
    move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
        let written = reader.take(data.len(), info, |first, second| {
            data[..first.len()].copy_from_slice(first);
            data[first.len()..first.len() + second.len()].copy_from_slice(second);
        });
        let volume = f32::from_bits(shared.volume.load(Ordering::Relaxed));
        if volume != 1.0 {
            for sample in &mut data[..written] {
                *sample *= volume;
            }
        }
        data[written..].fill(0.0);
        reader.count(written);
    }
}

/// The real-time callback for DoP: stamps the markers here rather than where
/// the words are made, so that no flush or underrun can break their
/// alternation. A DAC takes a broken one for PCM, at full level. There is no
/// volume, which would turn the words into noise, and all silence is DSD
/// silence.
fn dop_callback(
    mut reader: RingReader,
    rate: u32,
    paused: Arc<AtomicBool>,
) -> impl FnMut(&mut [i32], &cpal::OutputCallbackInfo) + Send + 'static {
    let mut lead_in_samples =
        (DOP_LEAD_IN.as_secs_f64() * f64::from(rate)) as usize * usize::from(DOP_CHANNELS);
    let mut marker_ix = 0;
    move |data: &mut [i32], info: &cpal::OutputCallbackInfo| {
        let lead = lead_in_samples.min(data.len());
        lead_in_samples -= lead;
        let (silent, rest) = data.split_at_mut(lead);
        let room = if paused.load(Ordering::Acquire) {
            0
        } else {
            rest.len()
        };
        let written = reader.take(room, info, |first, second| {
            for (slot, word) in rest.iter_mut().zip(first.iter().chain(second)) {
                // Exact: these are whole numbers that came from a `u16`
                *slot = *word as i32;
            }
        });
        silent.fill(i32::from(dsd::SILENCE_WORD));
        rest[written..].fill(i32::from(dsd::SILENCE_WORD));
        for frame in data.chunks_exact_mut(usize::from(DOP_CHANNELS)) {
            let marker = dsd::DOP_MARKERS[marker_ix];
            marker_ix = 1 - marker_ix;
            for sample in frame {
                let [_, _, high, low] = sample.to_be_bytes();
                *sample = i32::from_be_bytes([marker, high, low, 0]);
            }
        }
        reader.count(written);
    }
}

#[cfg(test)]
mod tests {
    use cpal::traits::StreamTrait as _;

    use std::io::Cursor;
    use std::time::Instant;

    use super::*;
    use crate::output::{Resampling, ResamplingWindow};
    use crate::test_wav::wav;

    fn next_event(events: &async_channel::Receiver<Event>) -> Event {
        next_event_within(events, Duration::from_secs(10))
    }

    fn next_event_within(events: &async_channel::Receiver<Event>, wait: Duration) -> Event {
        let deadline = Instant::now() + wait;
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
        assert!(matches!(next_event(&events).kind, EventKind::Ended { .. }));
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
        assert!(matches!(next_event(&events).kind, EventKind::Ended { .. }));
        assert!(sought.elapsed() < Duration::from_secs(5));
    }

    /// A rate no test wav uses, that the default device supports and is not
    /// running at.
    fn another_supported_rate() -> u32 {
        let current = default_device_rate();
        crate::output_devices()
            .unwrap()
            .into_iter()
            .find(crate::OutputDevice::is_default)
            .unwrap()
            .sample_rates()
            .iter()
            .copied()
            .find(|rate| *rate != current)
            .expect("the default device supports only one rate")
    }

    #[test]
    #[ignore = "needs an audio output device"]
    fn a_fixed_rate_resamples_every_track() {
        let (mut engine, events) = Engine::spawn();
        let fixed = another_supported_rate();
        let source = default_device_rate();
        engine.set_output(OutputSettings::new(None, Some(fixed)));
        engine.load(Track::new(Cursor::new(wav(source, 2, source, 0))));
        let EventKind::Started(format) = next_event(&events).kind else {
            panic!("expected Started");
        };
        assert_eq!(format.source_rate(), source);
        assert_eq!(format.output_rate(), fixed);
        assert!(format.is_resampled());
        // Waits for the device's rate to come back, which the next test's
        // engine would otherwise race.
        engine.shutdown();
    }

    #[test]
    #[ignore = "needs an audio output device"]
    fn new_settings_move_a_playing_track_and_keep_its_position() {
        let (mut engine, events) = Engine::spawn();
        let source = default_device_rate();
        engine.load(Track::new(Cursor::new(wav(source, 2, source * 20, 0))));
        assert!(matches!(next_event(&events).kind, EventKind::Started(_)));
        engine.seek(Duration::from_secs(10));
        thread::sleep(Duration::from_millis(200));

        let fixed = another_supported_rate();
        engine.set_output(OutputSettings::new(None, Some(fixed)));
        let EventKind::OutputChanged(format) = next_event(&events).kind else {
            panic!("expected OutputChanged");
        };
        assert_eq!(format.output_rate(), fixed);
        assert!(engine.position() >= Duration::from_secs(10));
        assert!(engine.position() < Duration::from_secs(12));
        engine.shutdown();
    }

    #[test]
    #[ignore = "needs an audio output device"]
    fn new_settings_keep_a_paused_track_paused() {
        let (mut engine, events) = Engine::spawn();
        let source = default_device_rate();
        engine.load(Track::new(Cursor::new(wav(source, 2, source * 20, 0))));
        assert!(matches!(next_event(&events).kind, EventKind::Started(_)));
        engine.pause();
        engine.seek(Duration::from_secs(10));

        engine.set_output(OutputSettings::new(None, Some(another_supported_rate())));
        assert!(matches!(
            next_event(&events).kind,
            EventKind::OutputChanged(_)
        ));
        thread::sleep(Duration::from_millis(300));
        let position = engine.position();
        assert!(position >= Duration::from_secs(10), "{position:?}");
        assert!(position < Duration::from_millis(10_100), "{position:?}");
        engine.shutdown();
    }

    #[test]
    #[ignore = "needs an audio output device"]
    #[cfg(target_os = "macos")]
    fn shutdown_restores_the_device_rate_after_new_settings() {
        let (mut engine, events) = Engine::spawn();
        let original = default_device_rate();
        let source = another_supported_rate();
        engine.load(Track::new(Cursor::new(wav(source, 2, source * 20, 0))));
        assert!(matches!(next_event(&events).kind, EventKind::Started(_)));
        engine.set_output(OutputSettings::new(None, Some(original)));
        assert!(matches!(
            next_event(&events).kind,
            EventKind::OutputChanged(_)
        ));
        engine.set_output(OutputSettings::new(None, Some(source)));
        assert!(matches!(
            next_event(&events).kind,
            EventKind::OutputChanged(_)
        ));
        engine.shutdown();
        assert_eq!(default_device_rate(), original);
    }

    /// The device these tests open by id: the one `HIPHILE_TEST_DEVICE` names,
    /// such as a virtual device, or else the default one.
    fn device_to_open_by_id() -> crate::OutputDevice {
        let name = std::env::var("HIPHILE_TEST_DEVICE").ok();
        crate::output_devices()
            .unwrap()
            .into_iter()
            .find(|device| match &name {
                Some(name) => device.name().trim() == name,
                None => device.is_default(),
            })
            .expect("no such device")
    }

    /// Sets the device's rate from outside the engine, as another app or
    /// Audio MIDI Setup does.
    fn set_rate_from_outside(id: &str, rate: u32) {
        let host = cpal::default_host();
        let device = host.device_by_id(&id.parse().unwrap()).unwrap();
        let mut config = device.default_output_config().unwrap().config();
        config.sample_rate = rate;
        let stream = device.build_output_stream(
            config,
            |data: &mut [f32], _: &cpal::OutputCallbackInfo| data.fill(0.0),
            |_| {},
            Some(RATE_SWITCH_TIMEOUT),
        );
        drop(stream.unwrap());
    }

    fn current_rate(id: &str) -> u32 {
        let host = cpal::default_host();
        let device = host.device_by_id(&id.parse().unwrap()).unwrap();
        device.default_output_config().unwrap().sample_rate()
    }

    #[test]
    #[ignore = "needs an audio output device"]
    fn a_device_chosen_by_id_plays_at_each_rate_asked_for() {
        let device = device_to_open_by_id();
        let id = device.id().unwrap().to_owned();
        let original = current_rate(&id);
        let other = *device
            .sample_rates()
            .iter()
            .find(|rate| **rate != original)
            .unwrap();

        let (mut engine, events) = Engine::spawn();
        engine.set_output(OutputSettings::new(Some(id.clone()), None));
        engine.load(Track::new(Cursor::new(wav(other, 2, other * 20, 0))));
        let EventKind::Started(format) = next_event(&events).kind else {
            panic!("expected Started");
        };
        assert_eq!(format.output_rate(), other);

        engine.set_output(OutputSettings::new(Some(id.clone()), Some(original)));
        let event = next_event(&events);
        let EventKind::OutputChanged(format) = event.kind else {
            panic!("expected OutputChanged, got {:?}", event.kind);
        };
        assert_eq!(format.output_rate(), original);
        // A late notice of the engine's own switch must not stop the track
        assert_no_event(&events);
        engine.shutdown();
    }

    #[test]
    #[ignore = "needs an audio output device"]
    fn a_rate_set_from_outside_is_kept_and_the_track_goes_on() {
        let device = device_to_open_by_id();
        let id = device.id().unwrap().to_owned();
        let original = current_rate(&id);
        let outside = *device
            .sample_rates()
            .iter()
            .find(|rate| **rate != original)
            .unwrap();

        let (mut engine, events) = Engine::spawn();
        engine.set_output(OutputSettings::new(Some(id.clone()), None));
        engine.load(Track::new(Cursor::new(wav(original, 2, original * 20, 0))));
        assert!(matches!(next_event(&events).kind, EventKind::Started(_)));
        engine.seek(Duration::from_secs(10));
        thread::sleep(Duration::from_millis(200));

        set_rate_from_outside(&id, outside);
        let event = next_event(&events);
        let EventKind::OutputChanged(format) = event.kind else {
            panic!("expected OutputChanged, got {:?}", event.kind);
        };
        assert_eq!(format.output_rate(), outside);
        assert!(format.is_resampled());
        assert!(engine.position() >= Duration::from_secs(10));
        assert!(engine.position() < Duration::from_secs(12));
        thread::sleep(Duration::from_millis(300));
        assert!(engine.position() > Duration::from_secs(10));
        assert_no_event(&events);

        engine.shutdown();
        let kept = current_rate(&id);
        set_rate_from_outside(&id, original);
        assert_eq!(kept, outside);
    }

    #[test]
    #[ignore = "needs an audio output device"]
    fn a_track_can_start_paused_partway_through() {
        let device = device_to_open_by_id();
        let id = device.id().unwrap().to_owned();
        let rate = current_rate(&id);

        let (mut engine, events) = Engine::spawn();
        engine.set_output(OutputSettings::new(Some(id), None));
        let track = Track::new(Cursor::new(wav(rate, 2, rate * 20, 0)))
            .start_at(Duration::from_secs(10))
            .start_paused(true);
        engine.load(track);
        assert!(matches!(next_event(&events).kind, EventKind::Started(_)));
        thread::sleep(Duration::from_millis(300));
        let position = engine.position();
        assert!(position >= Duration::from_secs(10), "{position:?}");
        assert!(position < Duration::from_millis(10_100), "{position:?}");

        engine.resume();
        thread::sleep(Duration::from_millis(300));
        assert!(engine.position() > position);
        engine.shutdown();
    }

    /// A loopback device, such as BlackHole, whose input hears its output.
    fn loopback_device() -> (cpal::Device, String) {
        let name = std::env::var("HIPHILE_TEST_DEVICE")
            .expect("name a loopback device in HIPHILE_TEST_DEVICE");
        let id = crate::output_devices()
            .unwrap()
            .into_iter()
            .find(|device| device.name().trim() == name)
            .expect("no such device")
            .id()
            .unwrap()
            .to_owned();
        let device = cpal::default_host()
            .device_by_id(&id.parse().unwrap())
            .unwrap();
        (device, id)
    }

    fn record(device: &cpal::Device, rate: u32) -> (cpal::Stream, Arc<Mutex<Vec<f32>>>) {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let config = cpal::StreamConfig {
            channels: 2,
            sample_rate: rate,
            buffer_size: cpal::BufferSize::Default,
        };
        let stream = device
            .build_input_stream(
                config,
                {
                    let heard = heard.clone();
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        let (frames, _) = data.as_chunks::<2>();
                        heard
                            .lock()
                            .unwrap()
                            .extend(frames.iter().map(|frame| frame[0]));
                    }
                },
                |_| {},
                Some(RATE_SWITCH_TIMEOUT),
            )
            .unwrap();
        stream.play().unwrap();
        (stream, heard)
    }

    /// 16-bit stereo WAV of a sine wave from sample `from`, up to `to`, so
    /// that files with consecutive ranges make up one wave between them.
    fn sine_wav(rate: u32, from: u32, to: u32) -> Vec<u8> {
        let mut bytes = wav(rate, 2, to - from, 0);
        let data = bytes.len() - (to - from) as usize * 4;
        let (frames, _) = bytes[data..].as_chunks_mut::<4>();
        for (frame, chunk) in (from..to).zip(frames) {
            let phase = f64::from(frame) * 997.0 * std::f64::consts::TAU / f64::from(rate);
            let sample = (phase.sin() * f64::from(i16::MAX / 2)) as i16;
            chunk[..2].copy_from_slice(&sample.to_le_bytes());
            chunk[2..].copy_from_slice(&sample.to_le_bytes());
        }
        bytes
    }

    fn sounding_part(heard: &[f32]) -> &[f32] {
        let start = heard.iter().position(|sample| sample.abs() > 0.1).unwrap();
        let end = heard.iter().rposition(|sample| sample.abs() > 0.1).unwrap();
        &heard[start..=end]
    }

    /// How far `heard` strays from one 997 Hz sine wave, of whatever level
    /// and phase fit it best. A sample missing, added or silent anywhere
    /// shifts or breaks the wave, wherever in its cycle that falls.
    fn departure_from_one_wave(heard: &[f32], rate: u32) -> f64 {
        let step = 997.0 * std::f64::consts::TAU / f64::from(rate);
        let phases = || (0..heard.len()).map(|ix| ix as f64 * step);
        let samples = || heard.iter().copied().map(f64::from);
        let fit = |wave: fn(f64) -> f64| {
            let sum: f64 = phases().zip(samples()).map(|(p, s)| wave(p) * s).sum();
            sum * 2.0 / heard.len() as f64
        };
        let (sines, cosines) = (fit(f64::sin), fit(f64::cos));
        phases()
            .zip(samples())
            .map(|(p, s)| (s - sines * p.sin() - cosines * p.cos()).abs())
            .fold(0.0, f64::max)
    }

    fn longest_silence(heard: &[f32]) -> usize {
        heard
            .split(|sample| sample.abs() > 0.01)
            .map(<[f32]>::len)
            .max()
            .unwrap_or(0)
    }

    /// Hands over `bytes` at `bytes_per_second`, as a slow connection does.
    /// Reads stop waiting once the track is canceled.
    fn slow_track(bytes: Vec<u8>, bytes_per_second: f64) -> Track {
        struct Slow {
            bytes: Cursor<Vec<u8>>,
            started: Instant,
            bytes_per_second: f64,
            canceled: Arc<AtomicBool>,
        }
        impl std::io::Read for Slow {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                loop {
                    if self.canceled.load(Ordering::Acquire) {
                        return Err(std::io::Error::other("canceled"));
                    }
                    let arrived =
                        (self.started.elapsed().as_secs_f64() * self.bytes_per_second) as u64;
                    let room = arrived.saturating_sub(self.bytes.position());
                    let end = self.bytes.position() >= self.bytes.get_ref().len() as u64;
                    if room > 0 || end {
                        let len = buffer
                            .len()
                            .min(usize::try_from(room).unwrap_or(usize::MAX));
                        return std::io::Read::read(&mut self.bytes, &mut buffer[..len]);
                    }
                    thread::sleep(Duration::from_millis(1));
                }
            }
        }
        impl std::io::Seek for Slow {
            fn seek(&mut self, to: std::io::SeekFrom) -> std::io::Result<u64> {
                std::io::Seek::seek(&mut self.bytes, to)
            }
        }
        let canceled = Arc::new(AtomicBool::new(false));
        let source = Slow {
            bytes: Cursor::new(bytes),
            started: Instant::now(),
            bytes_per_second,
            canceled: canceled.clone(),
        };
        Track::new(source)
            .extension("wav")
            .cancel_with(move || canceled.store(true, Ordering::Release))
    }

    /// The length of each silent stretch in `heard`, longer than the wave ever
    /// is near zero.
    fn gaps(heard: &[f32], rate: u32) -> Vec<usize> {
        let audible = rate as usize / 1_000;
        heard
            .split(|sample| sample.abs() > 0.01)
            .map(<[f32]>::len)
            .filter(|len| *len >= audible)
            .collect()
    }

    #[test]
    #[ignore = "needs a loopback device named in HIPHILE_TEST_DEVICE"]
    fn a_track_slower_than_it_plays_sounds_in_long_stretches() {
        let (device, id) = loopback_device();
        let rate = current_rate(&id);
        let (mut engine, events) = Engine::spawn();
        engine.set_output(OutputSettings::new(Some(id), None));
        let (stream, heard) = record(&device, rate);

        let seconds = 12;
        let bytes_per_second = f64::from(rate) * 4.0 * 0.4;
        let load = engine.load(slow_track(
            sine_wav(rate, 0, rate * seconds),
            bytes_per_second,
        ));
        let mut started = false;
        let mut waits = 0;
        let mut waited_out = 0;
        loop {
            let event = next_event_within(&events, Duration::from_secs(60));
            assert_eq!(event.load, load);
            match event.kind {
                EventKind::Started(_) => started = true,
                EventKind::Buffering(true) => waits += 1,
                EventKind::Buffering(false) => waited_out += 1,
                EventKind::Ended { .. } => break,
                other => panic!("{other:?}"),
            }
        }
        assert!(started);
        // At the start, and each time it runs out
        assert!(waits >= 2 && waits == waited_out, "{waits}, {waited_out}");
        thread::sleep(Duration::from_millis(200));
        drop(stream);
        engine.shutdown();

        let heard = heard.lock().unwrap();
        let sound = sounding_part(&heard);
        let gaps = gaps(sound, rate);
        assert!(gaps.len() <= 5, "{gaps:?}");
        // Four seconds of it take ten to arrive, twice the wait at the start
        let first = gaps.iter().find(|len| **len > rate as usize / 2);
        let first = *first.expect("no long wait") as f64 / f64::from(rate);
        assert!(first > 8.0, "the first wait took {first} seconds: {gaps:?}");
        let played = sound.iter().filter(|sample| sample.abs() > 0.01).count();
        let whole = (rate * seconds) as usize;
        assert!(played * 100 > whole * 95, "{played} of {whole}");
    }

    #[test]
    #[ignore = "needs a loopback device named in HIPHILE_TEST_DEVICE"]
    fn a_slow_next_track_waits_at_the_joint() {
        let (device, id) = loopback_device();
        let rate = current_rate(&id);
        let (mut engine, events) = Engine::spawn();
        engine.set_output(OutputSettings::new(Some(id), None));
        let (stream, heard) = record(&device, rate);

        let a = engine.load(Track::new(Cursor::new(sine_wav(rate, 0, rate))));
        let b = engine.set_next(slow_track(
            sine_wav(rate, rate, rate * 4),
            f64::from(rate) * 4.0 * 0.5,
        ));
        let mut ended = false;
        loop {
            let event = next_event_within(&events, Duration::from_secs(30));
            match event.kind {
                EventKind::Started(_) | EventKind::Buffering(_) => {}
                EventKind::Ended { next } if event.load == a => {
                    assert_eq!(next, Some(b));
                    ended = true;
                }
                EventKind::Ended { next: None } if event.load == b => break,
                other => panic!("{other:?}"),
            }
        }
        assert!(ended);
        thread::sleep(Duration::from_millis(200));
        drop(stream);
        engine.shutdown();

        let heard = heard.lock().unwrap();
        let sound = sounding_part(&heard);
        let silence = gaps(sound, rate);
        assert!(silence.len() <= 3, "{silence:?}");
        let wait = *silence
            .iter()
            .find(|len| **len > rate as usize / 2)
            .expect("no long wait");
        let before_the_wait = sound
            .windows(wait)
            .position(|window| window.iter().all(|sample| sample.abs() <= 0.01))
            .unwrap();
        let of_a = rate as usize;
        assert!(
            before_the_wait.abs_diff(of_a) < of_a / 10,
            "{before_the_wait} frames before the wait, against {of_a}"
        );
    }

    #[test]
    #[ignore = "needs a loopback device named in HIPHILE_TEST_DEVICE"]
    fn what_is_asked_while_a_track_opens_holds_once_it_has() {
        struct Delayed(Cursor<Vec<u8>>, Option<Duration>);
        impl std::io::Read for Delayed {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                if let Some(delay) = self.1.take() {
                    thread::sleep(delay);
                }
                std::io::Read::read(&mut self.0, buffer)
            }
        }
        impl std::io::Seek for Delayed {
            fn seek(&mut self, to: std::io::SeekFrom) -> std::io::Result<u64> {
                std::io::Seek::seek(&mut self.0, to)
            }
        }
        let device = device_to_open_by_id();
        let id = device.id().unwrap().to_owned();
        let rate = current_rate(&id);
        let (mut engine, events) = Engine::spawn();
        engine.set_output(OutputSettings::new(Some(id), None));

        let source = Delayed(
            Cursor::new(wav(rate, 2, rate * 10, 0)),
            Some(Duration::from_millis(500)),
        );
        engine.load(Track::new(source).extension("wav"));
        engine.pause();
        engine.seek(Duration::from_secs(5));
        assert!(matches!(next_event(&events).kind, EventKind::Started(_)));
        // Nor does a paused track count as waiting for its data
        assert_no_event(&events);
        let position = engine.position();
        assert!(position >= Duration::from_secs(5), "{position:?}");
        assert!(position < Duration::from_millis(5_100), "{position:?}");

        engine.resume();
        thread::sleep(Duration::from_millis(300));
        assert!(engine.position() > position);
        engine.shutdown();
    }

    #[test]
    #[ignore = "needs a loopback device named in HIPHILE_TEST_DEVICE"]
    fn a_track_waiting_for_its_data_is_still_controlled() {
        let device = device_to_open_by_id();
        let id = device.id().unwrap().to_owned();
        let rate = current_rate(&id);
        let (engine, events) = Engine::spawn();
        engine.set_output(OutputSettings::new(Some(id.clone()), None));

        // The header and some audio arrive quickly, the rest hardly at all
        let mut bytes = wav(rate, 2, rate * 60, 0);
        bytes.truncate(rate as usize);
        let load = engine.load(slow_start_track(bytes));
        assert!(matches!(next_event(&events).kind, EventKind::Started(_)));
        let waiting = next_event(&events);
        assert_eq!(waiting.load, load);
        assert!(matches!(waiting.kind, EventKind::Buffering(true)));

        // The next two take no time, and the wait is not reported as paused
        engine.pause();
        let done = next_event(&events);
        assert!(matches!(done.kind, EventKind::Buffering(false)), "{done:?}");
        engine.resume();
        let waiting = next_event(&events);
        assert!(matches!(waiting.kind, EventKind::Buffering(true)));

        // Moving to other settings means asking for data that will not come
        engine.seek(Duration::from_secs(50));
        thread::sleep(Duration::from_millis(50));
        assert_eq!(engine.position(), Duration::from_secs(50));
        engine.set_output(OutputSettings::new(
            Some(id),
            Some(another_supported_rate()),
        ));
        let moved = next_event(&events);
        assert!(
            matches!(moved.kind, EventKind::OutputChanged(_)),
            "{moved:?}"
        );

        engine.stop();
        shutdown_within_deadline(engine);
    }

    #[test]
    #[ignore = "needs a loopback device named in HIPHILE_TEST_DEVICE"]
    fn tracks_of_one_rate_join_without_a_gap() {
        let (device, id) = loopback_device();
        let rate = current_rate(&id);
        let (mut engine, events) = Engine::spawn();
        engine.set_output(OutputSettings::new(Some(id), None));
        let (stream, heard) = record(&device, rate);

        let (first, second, third) = (rate, rate * 2, rate * 3);
        let track = |from, to| Track::new(Cursor::new(sine_wav(rate, from, to)));
        let a = engine.load(track(0, first));
        let b = engine.set_next(track(first, second));
        let started = next_event(&events);
        assert_eq!(started.load, a);
        assert!(matches!(started.kind, EventKind::Started(_)));

        let ended = next_event(&events);
        assert_eq!(ended.load, a);
        assert!(matches!(ended.kind, EventKind::Ended { next } if next == Some(b)));
        let started = next_event(&events);
        assert_eq!(started.load, b);
        assert!(matches!(started.kind, EventKind::Started(_)));
        assert!(engine.position() < Duration::from_millis(100));

        let c = engine.set_next(track(second, third));
        let ended = next_event(&events);
        assert_eq!(ended.load, b);
        assert!(matches!(ended.kind, EventKind::Ended { next } if next == Some(c)));
        assert_eq!(next_event(&events).load, c);
        let ended = next_event(&events);
        assert_eq!(ended.load, c);
        assert!(matches!(ended.kind, EventKind::Ended { next: None }));
        thread::sleep(Duration::from_millis(200));
        drop(stream);
        engine.shutdown();

        let heard = heard.lock().unwrap();
        let wave = sounding_part(&heard);
        let departure = departure_from_one_wave(wave, rate);
        assert!(departure < 0.002, "strays from the wave by {departure}");
        let missing = (third as usize).abs_diff(wave.len());
        assert!(missing < 16, "{} frames against {third}", wave.len());
    }

    /// Tracks of one source rate that the device does not run at come out as
    /// one wave too, as the resampler carries on across the joint.
    #[test]
    #[ignore = "needs a loopback device named in HIPHILE_TEST_DEVICE"]
    fn resampled_tracks_of_one_rate_join_without_a_gap() {
        resampled_tracks_join_without_a_gap(Resampling::default());
    }

    #[test]
    #[ignore = "needs a loopback device named in HIPHILE_TEST_DEVICE"]
    fn resampled_tracks_join_without_a_gap_at_the_largest_fft() {
        resampled_tracks_join_without_a_gap(Resampling::new(16_384, ResamplingWindow::default()));
    }

    #[test]
    #[ignore = "needs a loopback device named in HIPHILE_TEST_DEVICE"]
    fn new_resampling_moves_a_resampled_track() {
        let (_, id) = loopback_device();
        let rate = current_rate(&id);
        let source = if rate == 44_100 { 48_000 } else { 44_100 };
        let (mut engine, events) = Engine::spawn();
        let settings = OutputSettings::new(Some(id), Some(rate));
        engine.set_output(settings.clone());
        engine.load(Track::new(Cursor::new(wav(source, 2, source * 20, 0))));
        assert!(matches!(next_event(&events).kind, EventKind::Started(_)));

        engine.set_output(
            settings.with_resampling(Resampling::new(4096, ResamplingWindow::default())),
        );
        assert!(matches!(
            next_event(&events).kind,
            EventKind::OutputChanged(_)
        ));
        engine.shutdown();
    }

    #[test]
    #[ignore = "needs a loopback device named in HIPHILE_TEST_DEVICE"]
    fn new_resampling_leaves_a_track_that_is_not_resampled_alone() {
        let (_, id) = loopback_device();
        let rate = current_rate(&id);
        let (mut engine, events) = Engine::spawn();
        let settings = OutputSettings::new(Some(id), Some(rate));
        engine.set_output(settings.clone());
        engine.load(Track::new(Cursor::new(wav(rate, 2, rate * 20, 0))));
        let EventKind::Started(format) = next_event(&events).kind else {
            panic!("expected Started");
        };
        assert!(!format.is_resampled());

        engine.set_output(
            settings.with_resampling(Resampling::new(4096, ResamplingWindow::default())),
        );
        thread::sleep(Duration::from_millis(300));
        assert!(events.try_recv().is_err());
        engine.shutdown();
    }

    fn resampled_tracks_join_without_a_gap(resampling: Resampling) {
        let (device, id) = loopback_device();
        let rate = current_rate(&id);
        let source = if rate == 44_100 { 48_000 } else { 44_100 };
        let (mut engine, events) = Engine::spawn();
        engine.set_output(OutputSettings::new(Some(id), Some(rate)).with_resampling(resampling));
        let (stream, heard) = record(&device, rate);

        let track = |from, to| Track::new(Cursor::new(sine_wav(source, from, to)));
        let a = engine.load(track(0, source));
        let b = engine.set_next(track(source, source * 2));
        assert_eq!(next_event(&events).load, a);
        let ended = next_event(&events);
        assert!(matches!(ended.kind, EventKind::Ended { next } if next == Some(b)));
        let position = engine.position();
        assert!(position < Duration::from_millis(100), "{position:?}");
        assert_eq!(next_event(&events).load, b);
        assert!(matches!(
            next_event(&events).kind,
            EventKind::Ended { next: None }
        ));
        thread::sleep(Duration::from_millis(200));
        drop(stream);
        engine.shutdown();

        let heard = heard.lock().unwrap();
        let wave = sounding_part(&heard);
        let missing = (rate as usize * 2).abs_diff(wave.len());
        assert!(missing < 128, "{} frames against {}", wave.len(), rate * 2);
        // Away from the far ends, where the resampler ramps in and out
        let margin = rate as usize / 10;
        let wave = &wave[margin..wave.len() - margin];
        let departure = departure_from_one_wave(wave, rate);
        assert!(departure < 0.002, "strays from the wave by {departure}");
    }

    /// A seek in the last moments of a track goes back to it from the next
    /// one, which then still follows from its start.
    #[test]
    #[ignore = "needs a loopback device named in HIPHILE_TEST_DEVICE"]
    fn a_seek_after_the_next_track_has_joined_stays_in_the_track() {
        let (device, id) = loopback_device();
        let rate = current_rate(&id);
        let (mut engine, events) = Engine::spawn();
        engine.set_output(OutputSettings::new(Some(id), None));
        let (_stream, _heard) = record(&device, rate);

        let track = |seconds| Track::new(Cursor::new(wav(rate, 2, rate * seconds, 0)));
        let a = engine.load(track(2));
        let b = engine.set_next(track(1));
        assert_eq!(next_event(&events).load, a);
        // Within what the ring buffer holds ahead of the end
        thread::sleep(Duration::from_millis(1_800));
        assert!(events.is_empty());
        let sought = Instant::now();
        engine.seek(Duration::from_secs(1));
        thread::sleep(Duration::from_millis(100));
        assert!(engine.position() >= Duration::from_secs(1));
        assert!(engine.position() < Duration::from_millis(1_500));

        let ended = next_event(&events);
        assert_eq!(ended.load, a);
        assert!(matches!(ended.kind, EventKind::Ended { next } if next == Some(b)));
        assert!(sought.elapsed() >= Duration::from_millis(900));
        assert_eq!(next_event(&events).load, b);
        let started_at = Instant::now();
        let ended = next_event(&events);
        assert_eq!(ended.load, b);
        assert!(matches!(ended.kind, EventKind::Ended { next: None }));
        assert!(started_at.elapsed() >= Duration::from_millis(900));
        engine.shutdown();
    }

    /// Replacing the next track after it has joined takes back its audio:
    /// the rest of the current track sounds in its place, and then the
    /// replacement. Silence in the taken-back track gives it away if not.
    #[test]
    #[ignore = "needs a loopback device named in HIPHILE_TEST_DEVICE"]
    fn a_next_track_replaced_after_it_joined_is_taken_back() {
        let (device, id) = loopback_device();
        let rate = current_rate(&id);
        let (mut engine, events) = Engine::spawn();
        engine.set_output(OutputSettings::new(Some(id), None));
        let (stream, heard) = record(&device, rate);

        let sine = |from, to| Track::new(Cursor::new(sine_wav(rate, from, to)));
        let a = engine.load(sine(0, rate * 2));
        engine.set_next(Track::new(Cursor::new(wav(rate, 2, rate * 5, 0))));
        assert_eq!(next_event(&events).load, a);
        thread::sleep(Duration::from_millis(1_800));
        assert!(events.is_empty());
        let c = engine.set_next(sine(rate * 2, rate * 3));

        let ended = next_event(&events);
        assert_eq!(ended.load, a);
        assert!(matches!(ended.kind, EventKind::Ended { next } if next == Some(c)));
        assert_eq!(next_event(&events).load, c);
        let ended = next_event(&events);
        assert_eq!(ended.load, c);
        assert!(matches!(ended.kind, EventKind::Ended { next: None }));
        thread::sleep(Duration::from_millis(200));
        drop(stream);
        engine.shutdown();

        let heard = heard.lock().unwrap();
        let wave = sounding_part(&heard);
        let tolerance = rate as usize / 20;
        let silence = longest_silence(wave);
        assert!(silence < tolerance, "{silence} silent frames");
        let total = rate as usize * 3;
        let frames = wave.len();
        assert!(
            total.abs_diff(frames) < tolerance,
            "{frames} frames against {total}"
        );
    }

    /// A source that has the start of a file and then cannot be read, like a
    /// connection lost right after a download started.
    fn track_that_breaks_after(bytes: Vec<u8>) -> Track {
        struct Breaks(Cursor<Vec<u8>>);
        impl std::io::Read for Breaks {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                match self.0.read(buffer)? {
                    0 => Err(std::io::Error::other("the connection was lost")),
                    read => Ok(read),
                }
            }
        }
        impl std::io::Seek for Breaks {
            fn seek(&mut self, to: std::io::SeekFrom) -> std::io::Result<u64> {
                self.0.seek(to)
            }
        }
        Track::new(Breaks(Cursor::new(bytes)))
    }

    /// A next track that breaks as it joins, before any of its audio has pushed
    /// the end of the current track out of the resampler. That end still
    /// sounds, and the next track fails at its turn, without having started.
    #[test]
    #[ignore = "needs a loopback device named in HIPHILE_TEST_DEVICE"]
    fn a_next_track_that_breaks_at_the_joint_lets_the_track_end() {
        let (device, id) = loopback_device();
        let rate = current_rate(&id);
        let source = if rate == 44_100 { 48_000 } else { 44_100 };
        let (mut engine, events) = Engine::spawn();
        engine.set_output(OutputSettings::new(Some(id), Some(rate)));
        let (stream, heard) = record(&device, rate);

        let a = engine.load(Track::new(Cursor::new(sine_wav(source, 0, source))));
        let mut start_of_b = sine_wav(source, source, source * 2);
        start_of_b.truncate(44 + 16);
        let b = engine.set_next(track_that_breaks_after(start_of_b));
        assert_eq!(next_event(&events).load, a);

        let ended = next_event(&events);
        assert_eq!(ended.load, a);
        assert!(matches!(ended.kind, EventKind::Ended { next } if next == Some(b)));
        let failed = next_event(&events);
        assert_eq!(failed.load, b);
        assert!(matches!(failed.kind, EventKind::Failed(_)));
        thread::sleep(Duration::from_millis(200));
        drop(stream);
        engine.shutdown();

        let heard = heard.lock().unwrap();
        let frames = sounding_part(&heard).len();
        assert!(
            (rate as usize).abs_diff(frames) < 128,
            "{frames} frames against {rate}"
        );
    }

    #[test]
    #[ignore = "needs a loopback device named in HIPHILE_TEST_DEVICE"]
    fn a_fixed_rate_at_the_top_of_the_device_range_plays() {
        let device = device_to_open_by_id();
        let id = device.id().unwrap().to_owned();
        let original = current_rate(&id);
        let top = *device.sample_rates().last().unwrap();
        let (mut engine, events) = Engine::spawn();
        engine.set_output(OutputSettings::new(Some(id.clone()), Some(top)));
        engine.load(Track::new(Cursor::new(wav(44_100, 2, 44_100 * 20, 0))));
        let EventKind::Started(format) = next_event(&events).kind else {
            panic!("expected Started");
        };
        assert_eq!(format.output_rate(), top);
        assert!(format.is_resampled());
        assert_eq!(current_rate(&id), top);
        thread::sleep(Duration::from_secs(1));
        assert!(engine.position() >= Duration::from_millis(500));
        assert_no_event(&events);
        engine.shutdown();
        assert_eq!(current_rate(&id), original);
    }

    #[test]
    #[ignore = "needs a loopback device named in HIPHILE_TEST_DEVICE"]
    fn a_next_track_at_another_rate_reopens_the_stream() {
        let device = device_to_open_by_id();
        let id = device.id().unwrap().to_owned();
        let rate = current_rate(&id);
        let other = *device
            .sample_rates()
            .iter()
            .find(|other| **other != rate)
            .unwrap();
        let (mut engine, events) = Engine::spawn();
        engine.set_output(OutputSettings::new(Some(id.clone()), None));
        let a = engine.load(Track::new(Cursor::new(wav(rate, 2, rate / 2, 0))));
        let b = engine.set_next(Track::new(Cursor::new(wav(other, 2, other / 2, 0))));
        assert_eq!(next_event(&events).load, a);
        let ended = next_event(&events);
        assert!(matches!(ended.kind, EventKind::Ended { next } if next == Some(b)));
        let started = next_event(&events);
        assert_eq!(started.load, b);
        let EventKind::Started(format) = started.kind else {
            panic!("expected Started");
        };
        assert_eq!(format.output_rate(), other);
        assert_eq!(current_rate(&id), other);
        let ended = next_event(&events);
        assert!(matches!(ended.kind, EventKind::Ended { next: None }));
        engine.shutdown();
        assert_eq!(current_rate(&id), rate);
    }

    #[test]
    #[ignore = "needs a loopback device named in HIPHILE_TEST_DEVICE"]
    fn stop_ends_the_wait_for_a_next_track_stuck_opening() {
        let device = device_to_open_by_id();
        let id = device.id().unwrap().to_owned();
        let rate = current_rate(&id);
        let (engine, events) = Engine::spawn();
        engine.set_output(OutputSettings::new(Some(id), None));
        let a = engine.load(Track::new(Cursor::new(wav(rate, 2, rate / 4, 0))));
        let b = engine.set_next(stalled_track());
        assert_eq!(next_event(&events).load, a);
        let ended = next_event(&events);
        assert!(matches!(ended.kind, EventKind::Ended { next } if next == Some(b)));
        let waiting = next_event(&events);
        assert_eq!(waiting.load, b);
        assert!(matches!(waiting.kind, EventKind::Buffering(true)));
        assert_no_event(&events);
        engine.stop();
        assert_no_event(&events);
        shutdown_within_deadline(engine);
    }

    #[test]
    #[ignore = "needs a loopback device named in HIPHILE_TEST_DEVICE"]
    fn a_next_track_that_cannot_open_fails_at_its_turn() {
        let device = device_to_open_by_id();
        let id = device.id().unwrap().to_owned();
        let rate = current_rate(&id);
        let (mut engine, events) = Engine::spawn();
        engine.set_output(OutputSettings::new(Some(id), None));
        let a = engine.load(Track::new(Cursor::new(wav(rate, 2, rate / 2, 0))));
        let b = engine.set_next(Track::new(Cursor::new(b"not audio".to_vec())));
        assert_eq!(next_event(&events).load, a);
        let ended = next_event(&events);
        assert_eq!(ended.load, a);
        assert!(matches!(ended.kind, EventKind::Ended { next } if next == Some(b)));
        let failed = next_event(&events);
        assert_eq!(failed.load, b);
        assert!(matches!(failed.kind, EventKind::Failed(_)));
        engine.shutdown();
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
        assert!(matches!(next_event(&events).kind, EventKind::Ended { .. }));
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
        assert!(matches!(next_event(events).kind, EventKind::Ended { .. }));
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
        // The next device plays at its own rate, and the old one still goes back
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
    fn only_a_stream_that_holds_its_device_reopens_when_the_default_changes() {
        use cpal::ErrorKind::{DeviceChanged, StreamInvalidated, Xrun};
        assert!(must_reopen(DeviceChanged, true));
        assert!(!must_reopen(DeviceChanged, false));
        assert!(must_reopen(StreamInvalidated, false));
        assert!(!must_reopen(Xrun, true));
    }

    #[test]
    fn samples_sound_until_the_device_has_played_them() {
        let (mut producer, consumer) = rtrb::RingBuffer::new(64);
        let shared = Arc::new(Shared::new());
        let sounding = Arc::new(AtomicBool::new(false));
        // 1 kHz stereo: one frame per millisecond
        let mut callback = output_callback(
            RingReader::new(
                consumer,
                2,
                1_000,
                shared.clone(),
                sounding.clone(),
                Arc::default(),
                Arc::default(),
            ),
            shared.clone(),
        );
        for sample in [0.5; 8] {
            producer.push(sample).unwrap();
        }
        let mut data = [1.0; 16];

        // Four frames go out at 0 ms and play from 10 ms, so until 14 ms
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

    #[test]
    fn samples_before_a_seek_are_dropped() {
        let (mut producer, consumer) = rtrb::RingBuffer::new(64);
        let shared = Arc::new(Shared::new());
        let flush_until = Arc::new(AtomicU64::new(0));
        let mut callback = output_callback(
            RingReader::new(
                consumer,
                2,
                1_000,
                shared.clone(),
                Arc::default(),
                flush_until.clone(),
                Arc::default(),
            ),
            shared.clone(),
        );
        for sample in [0.1; 6] {
            producer.push(sample).unwrap();
        }
        let mut data = [1.0; 2];
        callback(&mut data, &callback_info(0, 10));
        assert_eq!(data, [0.1, 0.1]);

        // Six samples were written before the seek, and the next two are new
        flush_until.store(6, Ordering::Release);
        for sample in [0.9; 2] {
            producer.push(sample).unwrap();
        }
        let mut data = [1.0; 4];
        callback(&mut data, &callback_info(2, 10));
        assert_eq!(data, [0.9, 0.9, 0.0, 0.0]);
        assert_eq!(shared.frames_played.load(Ordering::Relaxed), 2);
    }

    /// The position restarts at the first sample of the track that joined,
    /// wherever in a callback that falls, and only once.
    #[test]
    fn the_position_restarts_where_the_next_track_starts() {
        let (mut producer, consumer) = rtrb::RingBuffer::new(64);
        let shared = Arc::new(Shared::new());
        let next_at = Arc::new(AtomicU64::new(0));
        let mut callback = output_callback(
            RingReader::new(
                consumer,
                2,
                1_000,
                shared.clone(),
                Arc::default(),
                Arc::default(),
                next_at.clone(),
            ),
            shared.clone(),
        );
        let frames = || shared.frames_played.load(Ordering::Relaxed);
        for sample in [0.1; 8] {
            producer.push(sample).unwrap();
        }
        let mut data = [1.0; 4];
        callback(&mut data, &callback_info(0, 10));
        assert_eq!(frames(), 2);

        // The two frames left in the ring buffer end the track; the next one
        // starts at sample 8 and has two frames in there too.
        next_at.store(8, Ordering::Release);
        for sample in [0.9; 4] {
            producer.push(sample).unwrap();
        }
        let mut data = [1.0; 2];
        callback(&mut data, &callback_info(2, 10));
        assert_eq!(frames(), 3);
        assert_eq!(next_at.load(Ordering::Acquire), 8);

        let mut data = [1.0; 4];
        callback(&mut data, &callback_info(3, 10));
        assert_eq!(data, [0.1, 0.1, 0.9, 0.9]);
        assert_eq!(frames(), 1);
        assert_eq!(next_at.load(Ordering::Acquire), 0);

        let mut data = [1.0; 2];
        callback(&mut data, &callback_info(5, 10));
        assert_eq!(frames(), 2);
    }

    /// When the ring buffer ran empty before the next track joined, the
    /// restart waits for no sample to arrive.
    #[test]
    fn the_position_restarts_after_an_underrun_too() {
        let (mut producer, consumer) = rtrb::RingBuffer::new(64);
        let shared = Arc::new(Shared::new());
        let next_at = Arc::new(AtomicU64::new(0));
        let mut callback = output_callback(
            RingReader::new(
                consumer,
                2,
                1_000,
                shared.clone(),
                Arc::default(),
                Arc::default(),
                next_at.clone(),
            ),
            shared.clone(),
        );
        for sample in [0.1; 4] {
            producer.push(sample).unwrap();
        }
        let mut data = [1.0; 4];
        callback(&mut data, &callback_info(0, 10));
        next_at.store(4, Ordering::Release);
        callback(&mut data, &callback_info(2, 10));
        assert_eq!(shared.frames_played.load(Ordering::Relaxed), 0);
        assert_eq!(next_at.load(Ordering::Acquire), 0);
    }

    #[test]
    fn an_underrun_keeps_whole_frames() {
        let (mut producer, consumer) = rtrb::RingBuffer::new(64);
        let shared = Arc::new(Shared::new());
        let sounding = Arc::new(AtomicBool::new(false));
        let mut callback = output_callback(
            RingReader::new(
                consumer,
                2,
                1_000,
                shared.clone(),
                sounding,
                Arc::default(),
                Arc::default(),
            ),
            shared.clone(),
        );
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

    /// `start`, and then reads that wait until the track is canceled.
    fn slow_start_track(start: Vec<u8>) -> Track {
        let canceled = Arc::new(AtomicBool::new(false));
        struct Stuck {
            bytes: Cursor<Vec<u8>>,
            canceled: Arc<AtomicBool>,
        }
        impl std::io::Read for Stuck {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                let len = std::io::Read::read(&mut self.bytes, buffer)?;
                if len > 0 {
                    return Ok(len);
                }
                while !self.canceled.load(Ordering::Acquire) {
                    thread::sleep(Duration::from_millis(1));
                }
                Err(std::io::Error::other("canceled"))
            }
        }
        impl std::io::Seek for Stuck {
            fn seek(&mut self, to: std::io::SeekFrom) -> std::io::Result<u64> {
                std::io::Seek::seek(&mut self.bytes, to)
            }
        }
        let source = Stuck {
            bytes: Cursor::new(start),
            canceled: canceled.clone(),
        };
        Track::new(source)
            .extension("wav")
            .cancel_with(move || canceled.store(true, Ordering::Release))
    }

    fn assert_no_event(events: &async_channel::Receiver<Event>) {
        thread::sleep(Duration::from_millis(300));
        assert!(events.try_recv().is_err(), "unexpected event");
    }

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
        // Exactly one failure: the replacement's, not the canceled track's
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
    fn a_next_track_with_no_current_one_is_dropped() {
        let (engine, events) = Engine::spawn();
        engine.set_next(Track::new(Cursor::new(b"not audio".to_vec())));
        assert_no_event(&events);
        shutdown_within_deadline(engine);
    }

    #[test]
    fn a_device_that_is_not_connected_fails_the_track() {
        let (engine, events) = Engine::spawn();
        engine.set_output(OutputSettings::new(Some("not-a-device".to_owned()), None));
        engine.load(Track::new(Cursor::new(wav(44_100, 2, 4_410, 0))));
        assert!(matches!(
            next_event(&events).kind,
            EventKind::Failed(Error::OutputDeviceUnavailable)
        ));
    }

    #[test]
    fn reports_undecodable_tracks() {
        let (engine, events) = Engine::spawn();
        engine.load(Track::new(Cursor::new(b"not audio".to_vec())));
        assert!(matches!(next_event(&events).kind, EventKind::Failed(_)));
    }

    fn dop_settings(id: String, rate: dsd::DopRate) -> OutputSettings {
        let dop = dsd::Settings::new(rate, dsd::Filter::default(), dsd::Modulation::default());
        OutputSettings::new(Some(id), None).with_dop(Some(dop))
    }

    fn record_dop(device: &cpal::Device, rate: u32) -> (cpal::Stream, Arc<Mutex<Vec<[i32; 2]>>>) {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let config = cpal::StreamConfig {
            channels: 2,
            sample_rate: rate,
            buffer_size: cpal::BufferSize::Default,
        };
        let stream = device
            .build_input_stream(
                config,
                {
                    let heard = heard.clone();
                    move |data: &[i32], _: &cpal::InputCallbackInfo| {
                        heard
                            .lock()
                            .unwrap()
                            .extend_from_slice(data.as_chunks::<2>().0);
                    }
                },
                |_| {},
                Some(RATE_SWITCH_TIMEOUT),
            )
            .unwrap();
        stream.play().unwrap();
        (stream, heard)
    }

    /// The frames from when the stream started until it closed. A DoP frame is
    /// never zero, and a loopback device can sound old data after a stream
    /// ends, so the first zero is the end.
    fn stream_run(heard: &[[i32; 2]]) -> &[[i32; 2]] {
        let start = heard.iter().position(|frame| frame[0] != 0).unwrap();
        let len = heard[start..]
            .iter()
            .position(|frame| frame[0] == 0)
            .unwrap_or(heard.len() - start);
        &heard[start..start + len]
    }

    /// Asserts that `frames` are DoP throughout and returns their words on
    /// the left channel.
    fn dop_words(frames: &[[i32; 2]]) -> Vec<u16> {
        let mut last_marker = None;
        frames
            .iter()
            .enumerate()
            .map(|(index, [left, right])| {
                let [marker, high, low, rest] = left.to_be_bytes();
                let [right_marker, _, _, right_rest] = right.to_be_bytes();
                assert_eq!(rest, 0, "frame {index}");
                assert_eq!(right_rest, 0, "frame {index}");
                assert_eq!(marker, right_marker, "frame {index}");
                assert!(
                    dsd::DOP_MARKERS.contains(&marker),
                    "frame {index}: {marker:#x}"
                );
                assert_ne!(last_marker, Some(marker), "frame {index}");
                last_marker = Some(marker);
                u16::from_be_bytes([high, low])
            })
            .collect()
    }

    /// The amplitude and phase of `frequency` in each block of whole periods
    /// of `words`, read as DSD bits.
    fn tone_by_block(words: &[u16], frame_rate: u32, frequency: f64) -> Vec<(f64, f64)> {
        let per_period = f64::from(frame_rate) / frequency;
        let block = (per_period * 50.0).round() as usize;
        words
            .chunks_exact(block)
            .enumerate()
            .map(|(block_ix, words)| {
                let (mut re, mut im) = (0.0, 0.0);
                for (word_ix, word) in words.iter().enumerate() {
                    for bit_ix in 0..16usize {
                        let bit = if word >> (15 - bit_ix) & 1 == 1 {
                            1.0
                        } else {
                            -1.0
                        };
                        let at = (block_ix * block + word_ix) as f64 + bit_ix as f64 / 16.0;
                        let (sin, cos) = (std::f64::consts::TAU * at / per_period).sin_cos();
                        re += bit * cos;
                        im += bit * sin;
                    }
                }
                let count = (block * 16) as f64;
                (2.0 * re.hypot(im) / count, im.atan2(re))
            })
            .collect()
    }

    fn sounding_blocks(blocks: &[(f64, f64)]) -> &[(f64, f64)] {
        let start = blocks.iter().position(|block| block.0 > 0.2).unwrap();
        let end = blocks.iter().rposition(|block| block.0 > 0.2).unwrap();
        &blocks[start..=end]
    }

    #[test]
    #[ignore = "needs a loopback device named in HIPHILE_TEST_DEVICE"]
    fn dop_reaches_the_device_exactly_and_at_half_modulation() {
        let (device, id) = loopback_device();
        let frame_rate = 176_400;
        let (stream, heard) = record_dop(&device, frame_rate);
        let (mut engine, events) = Engine::spawn();
        engine.set_output(dop_settings(id, dsd::DopRate::Dsd64));
        let track = sine_wav(44_100, 0, 44_100);
        engine.load(Track::new(Cursor::new(track)));
        let EventKind::Started(format) = next_event(&events).kind else {
            panic!("expected Started");
        };
        assert_eq!(format.output_rate(), frame_rate);
        assert_eq!(format.dop_rate(), Some(dsd::DopRate::Dsd64));
        assert!(matches!(
            next_event(&events).kind,
            EventKind::Ended { next: None }
        ));
        thread::sleep(Duration::from_millis(300));
        drop(stream);
        engine.shutdown();

        let heard = heard.lock().unwrap();
        let words = dop_words(stream_run(&heard));
        let silence = usize::from(dsd::SILENCE_WORD);
        assert!(words.len() > frame_rate as usize / 2);
        assert_eq!(usize::from(words[0]), silence);

        let blocks = tone_by_block(&words, frame_rate, 997.0);
        let sounding = sounding_blocks(&blocks);
        // The blocks at either end are only partly the track
        for (level, _) in &sounding[1..sounding.len() - 1] {
            assert!((level - 0.25).abs() < 0.01, "{level}");
        }
        let seconds = sounding.len() as f64 * 50.0 / 997.0;
        assert!((seconds - 1.0).abs() < 0.1, "{seconds}");
    }

    #[test]
    #[ignore = "needs a loopback device named in HIPHILE_TEST_DEVICE"]
    fn dop_tracks_of_one_rate_join_without_a_gap() {
        let (device, id) = loopback_device();
        let frame_rate = 176_400;
        let (stream, heard) = record_dop(&device, frame_rate);
        let (mut engine, events) = Engine::spawn();
        engine.set_output(dop_settings(id, dsd::DopRate::Dsd64));
        engine.load(Track::new(Cursor::new(sine_wav(44_100, 0, 88_200))));
        assert!(matches!(next_event(&events).kind, EventKind::Started(_)));
        engine.set_next(Track::new(Cursor::new(sine_wav(44_100, 88_200, 176_400))));
        assert!(matches!(
            next_event(&events).kind,
            EventKind::Ended { next: Some(_) }
        ));
        assert!(matches!(next_event(&events).kind, EventKind::Started(_)));
        assert!(matches!(
            next_event(&events).kind,
            EventKind::Ended { next: None }
        ));
        thread::sleep(Duration::from_millis(300));
        drop(stream);
        engine.shutdown();

        let heard = heard.lock().unwrap();
        let words = dop_words(stream_run(&heard));
        let blocks = tone_by_block(&words, frame_rate, 997.0);
        let sounding = sounding_blocks(&blocks);
        let seconds = sounding.len() as f64 * 50.0 / 997.0;
        assert!((seconds - 4.0).abs() < 0.1, "{seconds}");
        let middle = &sounding[1..sounding.len() - 1];
        let first_phase = middle[0].1;
        for (level, phase) in middle {
            assert!((level - 0.25).abs() < 0.01, "{level}");
            // Whole periods per block, so a lost frame would turn the phase
            let turned = (phase - first_phase + std::f64::consts::PI)
                .rem_euclid(std::f64::consts::TAU)
                - std::f64::consts::PI;
            assert!(turned.abs() < 0.02, "{turned}");
        }
    }

    #[test]
    #[ignore = "needs a loopback device named in HIPHILE_TEST_DEVICE"]
    fn dop_keeps_its_frames_while_paused_and_after_a_seek() {
        let (device, id) = loopback_device();
        let frame_rate = 176_400;
        let (stream, heard) = record_dop(&device, frame_rate);
        let (mut engine, events) = Engine::spawn();
        engine.set_output(dop_settings(id, dsd::DopRate::Dsd64));
        engine.load(Track::new(Cursor::new(sine_wav(44_100, 0, 44_100 * 6))));
        assert!(matches!(next_event(&events).kind, EventKind::Started(_)));
        thread::sleep(Duration::from_millis(800));
        engine.pause();
        thread::sleep(Duration::from_millis(500));
        let position = engine.position();
        thread::sleep(Duration::from_millis(200));
        assert_eq!(engine.position(), position);
        engine.seek(Duration::from_secs(3));
        engine.resume();
        thread::sleep(Duration::from_millis(800));
        drop(stream);
        engine.shutdown();

        let heard = heard.lock().unwrap();
        let words = dop_words(stream_run(&heard));
        let silent_words = words
            .iter()
            .filter(|word| **word == dsd::SILENCE_WORD)
            .count();
        assert!(silent_words > frame_rate as usize / 4, "{silent_words}");
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "needs a loopback device named in HIPHILE_TEST_DEVICE"]
    fn dop_holds_the_device_and_lets_go_of_it() {
        let (device, id) = loopback_device();
        let (mut engine, events) = Engine::spawn();
        engine.set_output(dop_settings(id.clone(), dsd::DopRate::Dsd64));
        engine.load(Track::new(Cursor::new(sine_wav(44_100, 0, 44_100 * 3))));
        assert!(matches!(next_event(&events).kind, EventKind::Started(_)));
        assert_eq!(
            crate::exclusive_device::holder(&device),
            Some(std::process::id() as i32)
        );

        engine.stop();
        let deadline = Instant::now() + Duration::from_secs(2);
        while crate::exclusive_device::holder(&device) != Some(-1) {
            assert!(Instant::now() < deadline, "still held");
            thread::sleep(Duration::from_millis(10));
        }
        engine.shutdown();
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "needs a loopback device named in HIPHILE_TEST_DEVICE"]
    fn exclusive_pcm_holds_the_device_paused_too_and_lets_go_of_it() {
        let (device, id) = loopback_device();
        let own = Some(std::process::id() as i32);
        let (mut engine, events) = Engine::spawn();
        engine.set_output(OutputSettings::new(Some(id), None).with_exclusive(true));
        engine.load(Track::new(Cursor::new(sine_wav(96_000, 0, 96_000 * 6))));
        let EventKind::Started(format) = next_event(&events).kind else {
            panic!("expected Started");
        };
        assert_eq!(format.output_rate(), 96_000);
        assert_eq!(format.dop_rate(), None);
        assert_eq!(crate::exclusive_device::holder(&device), own);

        engine.pause();
        thread::sleep(Duration::from_millis(100));
        assert_eq!(crate::exclusive_device::holder(&device), own);
        engine.resume();

        engine.stop();
        let deadline = Instant::now() + Duration::from_secs(2);
        while crate::exclusive_device::holder(&device) != Some(-1) {
            assert!(Instant::now() < deadline, "still held");
            thread::sleep(Duration::from_millis(10));
        }
        engine.shutdown();
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "needs a loopback device named in HIPHILE_TEST_DEVICE"]
    fn exclusive_pcm_reaches_the_device_as_one_unbroken_wave() {
        let (device, id) = loopback_device();
        let rate = 48_000;
        let (stream, heard) = record(&device, rate);
        let (mut engine, events) = Engine::spawn();
        engine.set_output(OutputSettings::new(Some(id), Some(rate)).with_exclusive(true));
        engine.load(Track::new(Cursor::new(sine_wav(rate, 0, rate * 2))));
        assert!(matches!(next_event(&events).kind, EventKind::Started(_)));
        assert!(matches!(
            next_event(&events).kind,
            EventKind::Ended { next: None }
        ));
        thread::sleep(Duration::from_millis(200));
        drop(stream);
        engine.shutdown();

        let heard = heard.lock().unwrap();
        let wave = sounding_part(&heard);
        let departure = departure_from_one_wave(wave, rate);
        assert!(departure < 0.002, "strays from the wave by {departure}");
        let missing = (rate as usize * 2).abs_diff(wave.len());
        assert!(missing < 16, "{} frames against {}", wave.len(), rate * 2);
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "needs a loopback device named in HIPHILE_TEST_DEVICE"]
    fn turning_exclusive_mode_on_and_off_moves_a_track_where_it_was() {
        let (device, id) = loopback_device();
        let own = Some(std::process::id() as i32);
        let settings = OutputSettings::new(Some(id), None);
        let (mut engine, events) = Engine::spawn();
        engine.set_output(settings.clone());
        engine.load(Track::new(Cursor::new(sine_wav(44_100, 0, 44_100 * 8))));
        assert!(matches!(next_event(&events).kind, EventKind::Started(_)));
        assert_ne!(crate::exclusive_device::holder(&device), own);
        thread::sleep(Duration::from_millis(500));

        engine.set_output(settings.clone().with_exclusive(true));
        assert!(matches!(
            next_event(&events).kind,
            EventKind::OutputChanged(_)
        ));
        assert_eq!(crate::exclusive_device::holder(&device), own);
        assert!(engine.position() >= Duration::from_millis(400));

        engine.set_output(settings);
        assert!(matches!(
            next_event(&events).kind,
            EventKind::OutputChanged(_)
        ));
        assert_eq!(crate::exclusive_device::holder(&device), Some(-1));
        engine.shutdown();
    }

    #[test]
    #[ignore = "needs a loopback device named in HIPHILE_TEST_DEVICE"]
    fn turning_dop_on_moves_a_paused_track_there() {
        let (_, id) = loopback_device();
        let (mut engine, events) = Engine::spawn();
        engine.set_output(OutputSettings::new(Some(id.clone()), None));
        engine.load(Track::new(Cursor::new(sine_wav(44_100, 0, 44_100 * 6))));
        assert!(matches!(next_event(&events).kind, EventKind::Started(_)));
        thread::sleep(Duration::from_millis(300));
        engine.pause();
        engine.set_output(dop_settings(id, dsd::DopRate::Dsd128));
        let EventKind::OutputChanged(format) = next_event(&events).kind else {
            panic!("expected OutputChanged");
        };
        assert_eq!(format.dop_rate(), Some(dsd::DopRate::Dsd128));
        assert_eq!(format.output_rate(), 352_800);
        engine.shutdown();
    }

    /// That a tone rises from near silence to the full level.
    fn assert_fades_in(words: &[u16], frame_rate: u32) {
        let blocks = tone_by_block(words, frame_rate, 997.0);
        let first = blocks
            .iter()
            .find(|block| block.0 > 0.005)
            .expect("no sound");
        assert!(first.0 < 0.05, "starts at {}", first.0);
        let peak = blocks.iter().fold(0.0f64, |peak, block| peak.max(block.0));
        assert!(peak > 0.2, "only reaches {peak}");
    }

    #[test]
    #[ignore = "needs a loopback device named in HIPHILE_TEST_DEVICE"]
    fn new_dop_settings_while_paused_fade_in_again() {
        let (device, id) = loopback_device();
        let frame_rate = 176_400;
        let settings = |order| {
            let modulation = dsd::Modulation::default().with_order(order);
            let dop = dsd::Settings::new(dsd::DopRate::Dsd64, dsd::Filter::default(), modulation);
            OutputSettings::new(Some(id.clone()), None).with_dop(Some(dop))
        };
        let (mut engine, events) = Engine::spawn();
        engine.set_output(settings(dsd::Order::Seven));
        engine.load(Track::new(Cursor::new(sine_wav(44_100, 0, 44_100 * 12))));
        assert!(matches!(next_event(&events).kind, EventKind::Started(_)));
        thread::sleep(Duration::from_millis(300));
        engine.pause();
        engine.set_output(settings(dsd::Order::Five));
        assert!(matches!(
            next_event(&events).kind,
            EventKind::OutputChanged(_)
        ));
        let (stream, heard) = record_dop(&device, frame_rate);
        thread::sleep(Duration::from_millis(1_300));
        engine.resume();
        thread::sleep(Duration::from_millis(3_500));
        drop(stream);
        engine.shutdown();

        let heard = heard.lock().unwrap();
        assert_fades_in(&dop_words(stream_run(&heard)), frame_rate);
    }

    #[test]
    #[ignore = "needs a loopback device named in HIPHILE_TEST_DEVICE"]
    fn new_resampling_leaves_dop_alone_with_a_next_track_joined() {
        let (_, id) = loopback_device();
        let settings = dop_settings(id, dsd::DopRate::Dsd64);
        let (mut engine, events) = Engine::spawn();
        engine.set_output(settings.clone());
        engine.load(Track::new(Cursor::new(sine_wav(44_100, 0, 44_100 * 2))));
        engine.set_next(Track::new(Cursor::new(sine_wav(44_100, 0, 44_100 * 2))));
        assert!(matches!(next_event(&events).kind, EventKind::Started(_)));

        // The next track is joined only near the end of the first, so one change could miss it
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut fft_sizes = [4096, 8192].into_iter().cycle();
        let mut ended = false;
        while !ended {
            assert!(
                std::time::Instant::now() < deadline,
                "the tracks did not end"
            );
            let fft_size = fft_sizes.next().unwrap();
            engine.set_output(
                settings
                    .clone()
                    .with_resampling(Resampling::new(fft_size, ResamplingWindow::default())),
            );
            thread::sleep(Duration::from_millis(50));
            while let Ok(event) = events.try_recv() {
                assert!(!matches!(event.kind, EventKind::OutputChanged(_)));
                ended |= matches!(event.kind, EventKind::Ended { next: None });
            }
        }
        engine.shutdown();
    }

    #[test]
    #[ignore = "needs a loopback device named in HIPHILE_TEST_DEVICE"]
    fn dop_without_its_fade_starts_at_full_level() {
        let (device, id) = loopback_device();
        let frame_rate = 176_400;
        let (stream, heard) = record_dop(&device, frame_rate);
        let (mut engine, events) = Engine::spawn();
        engine.set_output(dop_settings(id, dsd::DopRate::Dsd64).with_dop_fade_in(false));
        engine.load(Track::new(Cursor::new(sine_wav(44_100, 0, 44_100))).fade_in(true));
        assert!(matches!(next_event(&events).kind, EventKind::Started(_)));
        assert!(matches!(
            next_event(&events).kind,
            EventKind::Ended { next: None }
        ));
        thread::sleep(Duration::from_millis(300));
        drop(stream);
        engine.shutdown();

        let heard = heard.lock().unwrap();
        let words = dop_words(stream_run(&heard));
        let lead_in = (DOP_LEAD_IN.as_secs_f64() * f64::from(frame_rate)) as usize;
        let (level, _) = tone_by_block(&words[lead_in..], frame_rate, 997.0)[0];
        assert!((level - 0.25).abs() < 0.01, "{level}");
    }

    #[test]
    #[ignore = "needs a loopback device named in HIPHILE_TEST_DEVICE"]
    fn dop_turned_on_while_a_track_opens_fades_it_in() {
        let (device, id) = loopback_device();
        let frame_rate = 176_400;
        let (stream, heard) = record_dop(&device, frame_rate);
        let (mut engine, events) = Engine::spawn();
        engine.set_output(OutputSettings::new(Some(id.clone()), None));
        let bytes = sine_wav(44_100, 0, 44_100 * 8);
        engine.load(slow_track(bytes, 44_100.0 * 4.0 * 4.0));
        engine.set_output(dop_settings(id, dsd::DopRate::Dsd64));
        assert!(matches!(next_event(&events).kind, EventKind::Started(_)));
        thread::sleep(Duration::from_millis(4_500));
        drop(stream);
        engine.shutdown();

        let heard = heard.lock().unwrap();
        assert_fades_in(&dop_words(stream_run(&heard)), frame_rate);
    }

    #[test]
    #[ignore = "needs a loopback device named in HIPHILE_TEST_DEVICE"]
    fn a_very_short_next_track_ends_on_dop() {
        let (_, id) = loopback_device();
        let (mut engine, events) = Engine::spawn();
        engine.set_output(dop_settings(id, dsd::DopRate::Dsd64));
        engine.load(Track::new(Cursor::new(sine_wav(44_100, 0, 22_050))));
        assert!(matches!(next_event(&events).kind, EventKind::Started(_)));
        engine.set_next(Track::new(Cursor::new(sine_wav(48_000, 0, 10))));
        while !matches!(
            next_event_within(&events, Duration::from_secs(8)).kind,
            EventKind::Ended { next: None }
        ) {}
        engine.shutdown();
    }

    #[test]
    #[ignore = "needs a loopback device named in HIPHILE_TEST_DEVICE"]
    fn a_pause_gets_through_while_the_encoder_is_busy() {
        let (_, id) = loopback_device();
        let filter = dsd::Filter::Sinc {
            length: dsd::SincLength::L2048,
            window: dsd::ResamplingWindow::BlackmanHarris2,
            cutoff: None,
        };
        let modulation = dsd::Modulation::default().with_order(dsd::Order::Nine);
        let dop = dsd::Settings::new(dsd::DopRate::Dsd256, filter, modulation);
        let (mut engine, events) = Engine::spawn();
        engine.set_output(OutputSettings::new(Some(id), None).with_dop(Some(dop)));
        let rate = 11_127;
        engine.load(Track::new(Cursor::new(sine_wav(rate, 0, rate * 60))));
        assert!(matches!(next_event(&events).kind, EventKind::Started(_)));
        thread::sleep(Duration::from_millis(2_500));
        engine.seek(Duration::from_secs(10));
        thread::sleep(Duration::from_millis(30));
        let asked = Instant::now();
        engine.pause();
        let mut last = engine.position();
        let mut steady_since = Instant::now();
        while steady_since.elapsed() < Duration::from_millis(300) {
            assert!(
                asked.elapsed() < Duration::from_secs(5),
                "never took effect"
            );
            thread::sleep(Duration::from_millis(5));
            let position = engine.position();
            if position != last {
                last = position;
                steady_since = Instant::now();
            }
        }
        let took = steady_since.duration_since(asked);
        assert!(took < Duration::from_secs(1), "{took:?}");
        engine.shutdown();
    }

    #[test]
    fn dop_frames_alternate_their_markers_over_every_kind_of_silence() {
        let (mut producer, consumer) = rtrb::RingBuffer::new(1_024);
        let shared = Arc::new(Shared::new());
        let paused = Arc::new(AtomicBool::new(false));
        // A rate low enough for the lead-in to be a single frame
        let mut callback = dop_callback(
            RingReader::new(
                consumer,
                2,
                1,
                shared.clone(),
                Arc::default(),
                Arc::default(),
                Arc::default(),
            ),
            1,
            paused.clone(),
        );
        for word in [0x1234u16, 0x5678, 0x9ABC, 0xDEF0] {
            producer.push(f32::from(word)).unwrap();
        }
        let mut data = [0; 10];
        callback(&mut data, &callback_info(0, 0));
        let silence = dsd::SILENCE_WORD.to_be_bytes();
        let frame = |marker: u8, left: u16, right: u16| {
            let [left_high, left_low] = left.to_be_bytes();
            let [right_high, right_low] = right.to_be_bytes();
            [
                i32::from_be_bytes([marker, left_high, left_low, 0]),
                i32::from_be_bytes([marker, right_high, right_low, 0]),
            ]
        };
        let idle = u16::from_be_bytes(silence);
        assert_eq!(data[..2], frame(0x05, idle, idle));
        assert_eq!(data[2..4], frame(0xFA, 0x1234, 0x5678));
        assert_eq!(data[4..6], frame(0x05, 0x9ABC, 0xDEF0));
        assert_eq!(data[6..8], frame(0xFA, idle, idle));
        assert_eq!(data[8..], frame(0x05, idle, idle));

        producer.push(f32::from(0x1111u16)).unwrap();
        producer.push(f32::from(0x2222u16)).unwrap();
        paused.store(true, Ordering::Release);
        let mut data = [0; 2];
        callback(&mut data, &callback_info(0, 0));
        assert_eq!(data, frame(0xFA, idle, idle));
        assert_eq!(shared.frames_played.load(Ordering::Relaxed), 2);

        paused.store(false, Ordering::Release);
        callback(&mut data, &callback_info(0, 0));
        assert_eq!(data, frame(0x05, 0x1111, 0x2222));
    }

    #[test]
    #[ignore = "needs a loopback device named in HIPHILE_TEST_DEVICE"]
    fn dop_keeps_one_stream_from_a_44_1_khz_track_to_a_48_khz_one() {
        let (device, id) = loopback_device();
        let frame_rate = 705_600;
        let (stream, heard) = record_dop(&device, frame_rate);
        let (mut engine, events) = Engine::spawn();
        engine.set_output(dop_settings(id, dsd::DopRate::Dsd256));
        engine.load(Track::new(Cursor::new(sine_wav(44_100, 0, 44_100))));
        let EventKind::Started(first) = next_event(&events).kind else {
            panic!("expected Started");
        };
        engine.set_next(Track::new(Cursor::new(sine_wav(48_000, 0, 48_000))));
        assert!(matches!(
            next_event(&events).kind,
            EventKind::Ended { next: Some(_) }
        ));
        let EventKind::Started(second) = next_event(&events).kind else {
            panic!("expected Started");
        };
        assert_eq!(first.output_rate(), frame_rate);
        assert_eq!(second.output_rate(), frame_rate);
        assert_eq!(second.source_rate(), 48_000);
        assert!(matches!(
            next_event(&events).kind,
            EventKind::Ended { next: None }
        ));
        thread::sleep(Duration::from_millis(300));
        drop(stream);
        engine.shutdown();

        let heard = heard.lock().unwrap();
        let words = dop_words(stream_run(&heard));
        let blocks = tone_by_block(&words, frame_rate, 997.0);
        let sounding = sounding_blocks(&blocks);
        let seconds = sounding.len() as f64 * 50.0 / 997.0;
        assert!((seconds - 2.0).abs() < 0.1, "{seconds}");
    }

    #[test]
    #[ignore = "needs a loopback device named in HIPHILE_TEST_DEVICE"]
    fn dop_can_run_at_the_rate_of_a_48_khz_source_when_asked() {
        let (device, id) = loopback_device();
        let frame_rate = 768_000;
        let (stream, heard) = record_dop(&device, frame_rate);
        let (mut engine, events) = Engine::spawn();
        let dop = dsd::Settings::new(
            dsd::DopRate::Dsd256,
            dsd::Filter::default(),
            dsd::Modulation::default(),
        )
        .with_matching_source_family(true);
        engine.set_output(OutputSettings::new(Some(id), None).with_dop(Some(dop)));
        engine.load(Track::new(Cursor::new(sine_wav(48_000, 0, 48_000))));
        let EventKind::Started(format) = next_event(&events).kind else {
            panic!("expected Started");
        };
        assert_eq!(format.output_rate(), frame_rate);
        assert!(matches!(
            next_event(&events).kind,
            EventKind::Ended { next: None }
        ));
        thread::sleep(Duration::from_millis(300));
        drop(stream);
        engine.shutdown();

        let heard = heard.lock().unwrap();
        assert!(dop_words(stream_run(&heard)).len() > frame_rate as usize);
    }

    /// Fails if any frame of the start of a track is lost: the tone has to
    /// begin at its first phase, right after the lead-in.
    fn dop_starts_exactly_after_the_lead_in(rate: dsd::DopRate, frame_rate: u32) {
        let (device, id) = loopback_device();
        let (stream, heard) = record_dop(&device, frame_rate);
        let (mut engine, events) = Engine::spawn();
        engine.set_output(dop_settings(id, rate));
        engine.load(Track::new(Cursor::new(sine_wav(44_100, 0, 44_100))));
        assert!(matches!(next_event(&events).kind, EventKind::Started(_)));
        assert!(matches!(
            next_event(&events).kind,
            EventKind::Ended { next: None }
        ));
        thread::sleep(Duration::from_millis(300));
        drop(stream);
        engine.shutdown();

        let heard = heard.lock().unwrap();
        let words = dop_words(stream_run(&heard));
        let lead_in = (DOP_LEAD_IN.as_secs_f64() * f64::from(frame_rate)) as usize;
        assert!(
            words[..lead_in]
                .iter()
                .all(|word| *word == dsd::SILENCE_WORD)
        );
        let blocks = tone_by_block(&words[lead_in..], frame_rate, 997.0);
        let (level, phase) = blocks[0];
        assert!((level - 0.25).abs() < 0.01, "{level}");
        let turned = phase - std::f64::consts::FRAC_PI_2;
        assert!(turned.abs() < 0.02, "{turned}");
    }

    #[test]
    #[ignore = "needs a loopback device named in HIPHILE_TEST_DEVICE"]
    fn dsd64_starts_exactly_after_the_lead_in() {
        dop_starts_exactly_after_the_lead_in(dsd::DopRate::Dsd64, 176_400);
    }

    #[test]
    #[ignore = "needs a loopback device named in HIPHILE_TEST_DEVICE"]
    fn dsd256_starts_exactly_after_the_lead_in() {
        dop_starts_exactly_after_the_lead_in(dsd::DopRate::Dsd256, 705_600);
    }
}
