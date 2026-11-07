#![expect(
    unsafe_code,
    reason = "cpal opens WASAPI in shared mode only, and COM has no safe binding"
)]

use std::collections::BTreeMap;
use std::ffi::c_void;
use std::mem::size_of;
use std::slice;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use cpal::traits::DeviceTrait;
use windows::Win32::Foundation::{
    CloseHandle, HANDLE, PROPERTYKEY, RPC_E_CHANGED_MODE, S_OK, WAIT_EVENT, WAIT_OBJECT_0,
    WAIT_TIMEOUT,
};
use windows::Win32::Media::Audio::{
    AUDCLNT_E_BUFFER_ERROR, AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED, AUDCLNT_E_BUFFER_TOO_LARGE,
    AUDCLNT_E_DEVICE_IN_USE, AUDCLNT_E_DEVICE_INVALIDATED, AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED,
    AUDCLNT_E_RESOURCES_INVALIDATED, AUDCLNT_E_SERVICE_NOT_RUNNING, AUDCLNT_E_UNSUPPORTED_FORMAT,
    AUDCLNT_SHAREMODE_EXCLUSIVE, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, DEVICE_STATE, EDataFlow, ERole,
    IAudioClient, IAudioRenderClient, IMMDevice, IMMDeviceEnumerator, IMMNotificationClient,
    IMMNotificationClient_Impl, MMDeviceEnumerator, WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
    WAVEFORMATEXTENSIBLE_0, eConsole, eRender,
};
use windows::Win32::Media::KernelStreaming::{KSDATAFORMAT_SUBTYPE_PCM, WAVE_FORMAT_EXTENSIBLE};
use windows::Win32::Media::Multimedia::KSDATAFORMAT_SUBTYPE_IEEE_FLOAT;
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
    CoUninitialize,
};
use windows::Win32::System::Threading::{
    AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, CreateEventW, SetEvent,
    WaitForMultipleObjects,
};
use windows::core::{Error as WindowsError, PCWSTR, implement, w};

use super::sample_layout::{SampleLayout, aligned_period, speaker_mask};
use crate::Error;
use crate::output::{ConfigRange, STANDARD_RATES, standard_rates};

/// How long a playing device may go without asking for audio before it counts
/// as gone. A removed device does not always signal its event again.
const SILENT_DEVICE_TIMEOUT_MS: u32 = 2_000;

/// How many times in a row a device may answer that its buffer is not ready
/// before it counts as failed. One such answer only means waiting for the next
/// event.
const MOST_BUFFER_ERRORS: u32 = 100;

const STEREO: u16 = 2;
const BITS_PER_BYTE: u16 = 8;

/// The last ranges each device showed. A driver may answer nothing while it is
/// held, by this process as well, which says nothing about what it can do.
static KNOWN_RANGES: Mutex<BTreeMap<String, Vec<ConfigRange>>> = Mutex::new(BTreeMap::new());

/// What `device` can run at in exclusive mode, which is often more than its
/// shared mode offers.
pub(crate) fn pcm_ranges(device: &cpal::Device) -> Result<Vec<ConfigRange>, Error> {
    let id = endpoint_id(device)?;
    let probed = probe_ranges(&id);
    let remembered = {
        let mut known = KNOWN_RANGES.lock().unwrap_or_else(PoisonError::into_inner);
        if let Ok(ranges) = &probed
            && !ranges.is_empty()
        {
            known.insert(id.clone(), ranges.clone());
        }
        known.get(&id).filter(|ranges| !ranges.is_empty()).cloned()
    };
    if let Some(ranges) = remembered {
        return Ok(ranges);
    }
    let ranges = probed?;
    let _com = Com::new()?;
    refusal(&endpoint(&id)?)?;
    Ok(ranges)
}

/// The exclusive rates of `device` for showing, probed once, as the devices
/// are listed every few seconds and each probe asks the driver up to a hundred
/// times. [`pcm_ranges`] brings them up to date whenever a stream opens.
pub(crate) fn listed_rates(device: &cpal::Device, _shared_rates: &[u32]) -> Vec<u32> {
    let Ok(id) = endpoint_id(device) else {
        return Vec::new();
    };
    let remembered = KNOWN_RANGES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&id)
        .cloned();
    let ranges = match remembered {
        Some(ranges) => ranges,
        None => {
            let Ok(ranges) = probe_ranges(&id) else {
                return Vec::new();
            };
            KNOWN_RANGES
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(id, ranges.clone());
            ranges
        }
    };
    standard_rates(&ranges)
}

/// Why no format at all was taken, if that is not about the formats. Their
/// check is not documented to report a device that is in use or closed to
/// exclusive streams, so this takes the device for a moment to find out.
fn refusal(device: &IMMDevice) -> Result<(), Error> {
    let client = activate(device)?;
    let mix = mix_format(&client)?;
    let format = wave_format(
        SampleLayout::Int16,
        mix.rate,
        mix.count,
        mix.mask.unwrap_or_else(|| speaker_mask(mix.count)),
    );
    let mut period = 0;
    // SAFETY: Writes one value, which outlives the call
    unsafe { client.GetDevicePeriod(Some(&mut period), None) }.map_err(opening_error)?;
    match initialize(&client, &format, period) {
        Err(error)
            if [
                AUDCLNT_E_DEVICE_IN_USE,
                AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED,
            ]
            .contains(&error.code()) =>
        {
            Err(opening_error(error))
        }
        _ => Ok(()),
    }
}

fn probe_ranges(id: &str) -> Result<Vec<ConfigRange>, Error> {
    let _com = Com::new()?;
    let device = endpoint(id)?;
    let client = activate(&device)?;
    let mix = mix_format(&client)?;
    let mut ranges = Vec::new();
    for channels in channels_to_probe(mix) {
        for rate in STANDARD_RATES {
            if supported_format(&client, rate, channels, mix)?.is_some() {
                ranges.push(ConfigRange {
                    channels,
                    min_rate: rate,
                    max_rate: rate,
                });
            }
        }
    }
    Ok(ranges)
}

fn channels_to_probe(mix: MixFormat) -> Vec<u16> {
    if mix.count == STEREO {
        vec![STEREO]
    } else {
        vec![mix.count, STEREO]
    }
}

/// A device stream that has the device to itself, for as long as it exists.
pub(crate) struct Stream {
    commands: mpsc::Sender<Command>,
    wake: Arc<Event>,
    thread: Option<thread::JoinHandle<()>>,
}

/// What the stream's thread waits on, besides its device.
struct Control {
    commands: mpsc::Receiver<Command>,
    wake: Arc<Event>,
    default_changed: Arc<Event>,
}

enum Command {
    Play,
    Pause,
    Close,
}

impl Stream {
    /// Fails as soon as the device cannot be taken. Nothing plays until
    /// [`Self::play`]. With `follows_default`, the system's default device
    /// changing ends the stream as invalidated, for it to open on the new one.
    pub(crate) fn open(
        device: &cpal::Device,
        config: cpal::StreamConfig,
        follows_default: bool,
        render: impl FnMut(&mut [f32], &cpal::OutputCallbackInfo) + Send + 'static,
        on_error: impl FnMut(cpal::Error) + Send + 'static,
    ) -> Result<Self, Error> {
        let id = endpoint_id(device)?;
        let wake = Arc::new(Event::new().map_err(opening_error)?);
        let default_changed = Arc::new(Event::new().map_err(opening_error)?);
        let (commands, command_receiver) = mpsc::channel();
        let (opened, opened_receiver) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("hiphile-wasapi".to_owned())
            .spawn({
                let control = Control {
                    commands: command_receiver,
                    wake: Arc::clone(&wake),
                    default_changed,
                };
                move || {
                    run(
                        &id,
                        config,
                        follows_default,
                        render,
                        on_error,
                        &opened,
                        &control,
                    );
                }
            })
            .map_err(|error| Error::Output(error.to_string()))?;
        match opened_receiver.recv() {
            Ok(Ok(())) => Ok(Self {
                commands,
                wake,
                thread: Some(thread),
            }),
            Ok(Err(error)) => Err(error),
            Err(error) => Err(Error::Output(error.to_string())),
        }
    }

    pub(crate) fn play(&self) {
        self.send(Command::Play);
    }

    pub(crate) fn pause(&self) {
        self.send(Command::Pause);
    }

    /// A thread that has ended has reported why through its error callback.
    fn send(&self, command: Command) {
        self.commands.send(command).ok();
        self.wake.signal();
    }
}

impl Drop for Stream {
    /// Waits for the device to be free, as the next stream opens right after.
    fn drop(&mut self) {
        self.send(Command::Close);
        if let Some(thread) = self.thread.take() {
            thread.join().ok();
        }
    }
}

fn run(
    id: &str,
    config: cpal::StreamConfig,
    follows_default: bool,
    mut render: impl FnMut(&mut [f32], &cpal::OutputCallbackInfo),
    mut on_error: impl FnMut(cpal::Error),
    opened: &mpsc::Sender<Result<(), Error>>,
    control: &Control,
) {
    let opening =
        Com::new().and_then(|com| Session::open(id, config).map(|session| (com, session)));
    let (_com, mut session) = match opening {
        Ok(opening) => {
            opened.send(Ok(())).ok();
            opening
        }
        Err(error) => {
            opened.send(Err(error)).ok();
            return;
        }
    };
    // Without the watch the stream still plays, on the device it opened on
    let _watch = follows_default
        .then(|| DefaultDeviceWatch::new(&control.default_changed).ok())
        .flatten();
    let _priority = ProAudioPriority::new();
    if let Err(error) = session.serve(&mut render, control) {
        on_error(stream_error(&error));
    }
}

/// An open, exclusive audio client. Lives on the thread that opened it.
struct Session {
    client: IAudioClient,
    renderer: IAudioRenderClient,
    event: Event,
    layout: SampleLayout,
    frames: u32,
    samples: Vec<f32>,
    latency: Duration,
    started: Instant,
    buffer_errors: u32,
}

impl Session {
    fn open(id: &str, config: cpal::StreamConfig) -> Result<Self, Error> {
        let rate = config.sample_rate;
        let device = endpoint(id)?;
        let first = activate(&device)?;
        let mix = mix_format(&first)?;
        let Some((layout, format)) = supported_format(&first, rate, config.channels, mix)? else {
            refusal(&device)?;
            return Err(Error::UnsupportedOutputFormat);
        };
        let mut period = 0;
        // SAFETY: Writes one value, which outlives the call
        unsafe { first.GetDevicePeriod(Some(&mut period), None) }.map_err(opening_error)?;
        let client = match initialize(&first, &format, period) {
            Ok(()) => first,
            Err(error) if error.code() == AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED => {
                // SAFETY: This failure leaves the size that would have fit here
                let frames = unsafe { first.GetBufferSize() }.map_err(opening_error)?;
                // A client that failed to initialize cannot try again, and has
                // to be released before the next one activates
                drop(first);
                let client = activate(&device)?;
                initialize(&client, &format, aligned_period(frames, rate))
                    .map_err(opening_error)?;
                client
            }
            Err(error) => return Err(opening_error(error)),
        };
        let event = Event::new().map_err(opening_error)?;
        // SAFETY: The client is initialized for events. The session holds the
        // event and drops the client first.
        unsafe { client.SetEventHandle(event.0) }.map_err(opening_error)?;
        // SAFETY: The client is initialized
        let frames = unsafe { client.GetBufferSize() }.map_err(opening_error)?;
        // SAFETY: The client is initialized
        let renderer =
            unsafe { client.GetService::<IAudioRenderClient>() }.map_err(opening_error)?;
        Ok(Self {
            client,
            renderer,
            event,
            layout,
            frames,
            samples: vec![0.0; frames as usize * usize::from(config.channels)],
            // One buffer playing and the one just written
            latency: Duration::from_secs_f64(2.0 * f64::from(frames) / f64::from(rate)),
            started: Instant::now(),
            buffer_errors: 0,
        })
    }

    /// Plays until told to close, or until the device fails.
    fn serve(
        &mut self,
        render: &mut impl FnMut(&mut [f32], &cpal::OutputCallbackInfo),
        control: &Control,
    ) -> Result<(), WindowsError> {
        // The wait favors the first, and a device that outruns the rendering
        // must not keep the stream from closing
        let handles = [control.wake.0, control.default_changed.0, self.event.0];
        let default_changed = WAIT_EVENT(WAIT_OBJECT_0.0 + 1);
        let device_signaled = WAIT_EVENT(WAIT_OBJECT_0.0 + 2);
        let mut is_playing = false;
        loop {
            // SAFETY: The handles live as long as `self` and `control`
            let signaled =
                unsafe { WaitForMultipleObjects(&handles, false, SILENT_DEVICE_TIMEOUT_MS) };
            if signaled == device_signaled {
                if is_playing {
                    self.write_buffer(render)?;
                }
            } else if signaled == default_changed {
                return Err(AUDCLNT_E_DEVICE_INVALIDATED.into());
            } else if signaled == WAIT_OBJECT_0 {
                while let Ok(command) = control.commands.try_recv() {
                    match command {
                        Command::Play if !is_playing => {
                            self.start(render)?;
                            is_playing = true;
                        }
                        Command::Pause if is_playing => {
                            // SAFETY: The client is initialized
                            unsafe { self.client.Stop() }?;
                            is_playing = false;
                        }
                        Command::Play | Command::Pause => {}
                        Command::Close => {
                            if is_playing {
                                // SAFETY: The client is initialized
                                unsafe { self.client.Stop() }?;
                            }
                            return Ok(());
                        }
                    }
                }
            } else if signaled == WAIT_TIMEOUT {
                if is_playing {
                    return Err(AUDCLNT_E_DEVICE_INVALIDATED.into());
                }
            } else {
                return Err(WindowsError::from_thread());
            }
        }
    }

    /// An exclusive stream plays what its buffer holds from the moment it
    /// starts, so that has to be there first. After a pause, what the device
    /// took before stopping left room to fill, or else it is still all there.
    fn start(
        &mut self,
        render: &mut impl FnMut(&mut [f32], &cpal::OutputCallbackInfo),
    ) -> Result<(), WindowsError> {
        self.write_buffer(render)?;
        // SAFETY: The client is initialized
        unsafe { self.client.Start() }
    }

    fn write_buffer(
        &mut self,
        render: &mut impl FnMut(&mut [f32], &cpal::OutputCallbackInfo),
    ) -> Result<(), WindowsError> {
        // SAFETY: Asks for the whole buffer, which is what an exclusive stream
        // has free each time its event fires
        let buffer = match unsafe { self.renderer.GetBuffer(self.frames) } {
            Ok(buffer) => buffer,
            // The buffer has yet to be played
            Err(error) if error.code() == AUDCLNT_E_BUFFER_TOO_LARGE => return Ok(()),
            Err(error)
                if error.code() == AUDCLNT_E_BUFFER_ERROR
                    && self.buffer_errors < MOST_BUFFER_ERRORS =>
            {
                self.buffer_errors += 1;
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        self.buffer_errors = 0;
        let callback = cpal::StreamInstant::ZERO
            .checked_add(self.started.elapsed())
            .unwrap_or(cpal::StreamInstant::ZERO);
        let playback = callback.checked_add(self.latency).unwrap_or(callback);
        render(
            &mut self.samples,
            &cpal::OutputCallbackInfo::new(cpal::OutputStreamTimestamp { callback, playback }),
        );
        let len = self.samples.len() * self.layout.bytes();
        // SAFETY: GetBuffer gave `frames` frames of this format, which is `len`
        // bytes, until ReleaseBuffer below
        let bytes = unsafe { slice::from_raw_parts_mut(buffer, len) };
        self.layout.write(&self.samples, bytes);
        // SAFETY: Releases what GetBuffer gave, all of it written
        unsafe { self.renderer.ReleaseBuffer(self.frames, 0) }
    }
}

fn initialize(
    client: &IAudioClient,
    format: &WAVEFORMATEXTENSIBLE,
    period: i64,
) -> Result<(), WindowsError> {
    // SAFETY: The format is read during the call. An event-driven exclusive
    // stream is required to pass its period as the buffer duration too.
    unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_EXCLUSIVE,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            period,
            period,
            &raw const format.Format,
            None,
        )
    }
}

fn opening_error(error: WindowsError) -> Error {
    let code = error.code();
    if code == AUDCLNT_E_DEVICE_IN_USE {
        Error::DeviceHeld
    } else if code == AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED {
        Error::ExclusiveNotAllowed
    } else if code == AUDCLNT_E_UNSUPPORTED_FORMAT {
        Error::UnsupportedOutputFormat
    } else if code == AUDCLNT_E_DEVICE_INVALIDATED {
        Error::OutputDeviceUnavailable
    } else {
        Error::ExclusiveOutput(error.to_string())
    }
}

fn stream_error(error: &WindowsError) -> cpal::Error {
    let code = error.code();
    let kind = if code == AUDCLNT_E_DEVICE_INVALIDATED || code == AUDCLNT_E_RESOURCES_INVALIDATED {
        cpal::ErrorKind::StreamInvalidated
    } else {
        cpal::ErrorKind::BackendError
    };
    cpal::Error::with_message(kind, error.to_string())
}

fn endpoint_id(device: &cpal::Device) -> Result<String, Error> {
    let id = device.id().map_err(|_| Error::OutputDeviceUnavailable)?;
    Ok(id.id().to_owned())
}

fn endpoint(id: &str) -> Result<IMMDevice, Error> {
    let wide: Vec<u16> = id.encode_utf16().chain([0]).collect();
    let enumerator = enumerator().map_err(opening_error)?;
    // SAFETY: `wide` is a terminated string that outlives the call
    unsafe { enumerator.GetDevice(PCWSTR(wide.as_ptr())) }
        .map_err(|_| Error::OutputDeviceUnavailable)
}

fn enumerator() -> Result<IMMDeviceEnumerator, WindowsError> {
    // SAFETY: COM is initialized on this thread
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
}

fn activate(device: &IMMDevice) -> Result<IAudioClient, Error> {
    // SAFETY: No activation parameters, and COM is initialized on this thread
    unsafe { device.Activate(CLSCTX_ALL, None) }.map_err(opening_error)
}

#[derive(Clone, Copy)]
struct MixFormat {
    rate: u32,
    count: u16,
    /// The system's speaker layout, if the mix format tells.
    mask: Option<u32>,
}

fn mix_format(client: &IAudioClient) -> Result<MixFormat, Error> {
    // SAFETY: The format is ours to read and to free. Its tag says when it is
    // the longer structure.
    unsafe {
        let format = client.GetMixFormat().map_err(opening_error)?;
        let rate = (*format).nSamplesPerSec;
        let count = (*format).nChannels;
        let is_extensible = u32::from((*format).wFormatTag) == WAVE_FORMAT_EXTENSIBLE;
        let mask = is_extensible
            .then(|| std::ptr::read_unaligned(format.cast::<WAVEFORMATEXTENSIBLE>()).dwChannelMask);
        CoTaskMemFree(Some(format.cast::<c_void>().cast_const()));
        Ok(MixFormat { rate, count, mask })
    }
}

/// The preferred layout the driver takes at this rate and channel count. An
/// error is about the device as a whole, not this format. Other answers count
/// as refusing the format, as a driver may answer an odd one in its own way.
fn supported_format(
    client: &IAudioClient,
    rate: u32,
    channels: u16,
    mix: MixFormat,
) -> Result<Option<(SampleLayout, WAVEFORMATEXTENSIBLE)>, Error> {
    let mask = mix
        .mask
        .filter(|_| channels == mix.count)
        .unwrap_or_else(|| speaker_mask(channels));
    for layout in SampleLayout::PREFERRED {
        let format = wave_format(layout, rate, channels, mask);
        // SAFETY: The format is read during the call. Exclusive mode writes no
        // closest match.
        let result = unsafe {
            client.IsFormatSupported(AUDCLNT_SHAREMODE_EXCLUSIVE, &raw const format.Format, None)
        };
        if result == S_OK {
            return Ok(Some((layout, format)));
        }
        if [
            AUDCLNT_E_DEVICE_IN_USE,
            AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED,
            AUDCLNT_E_DEVICE_INVALIDATED,
            AUDCLNT_E_RESOURCES_INVALIDATED,
            AUDCLNT_E_SERVICE_NOT_RUNNING,
        ]
        .contains(&result)
        {
            return Err(opening_error(result.into()));
        }
    }
    Ok(None)
}

fn wave_format(layout: SampleLayout, rate: u32, channels: u16, mask: u32) -> WAVEFORMATEXTENSIBLE {
    let container_bits = layout.bytes() as u16 * BITS_PER_BYTE;
    let block_align = channels * layout.bytes() as u16;
    WAVEFORMATEXTENSIBLE {
        Format: WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_EXTENSIBLE as u16,
            nChannels: channels,
            nSamplesPerSec: rate,
            nAvgBytesPerSec: rate * u32::from(block_align),
            nBlockAlign: block_align,
            wBitsPerSample: container_bits,
            cbSize: (size_of::<WAVEFORMATEXTENSIBLE>() - size_of::<WAVEFORMATEX>()) as u16,
        },
        Samples: WAVEFORMATEXTENSIBLE_0 {
            wValidBitsPerSample: layout.valid_bits(),
        },
        dwChannelMask: mask,
        SubFormat: if layout.is_float() {
            KSDATAFORMAT_SUBTYPE_IEEE_FLOAT
        } else {
            KSDATAFORMAT_SUBTYPE_PCM
        },
    }
}

/// Signals `changed` when the system's default output device changes, while
/// it lives. Dropping waits for a callback in progress.
struct DefaultDeviceWatch {
    enumerator: IMMDeviceEnumerator,
    client: IMMNotificationClient,
}

impl DefaultDeviceWatch {
    fn new(changed: &Arc<Event>) -> Result<Self, WindowsError> {
        let enumerator = enumerator()?;
        let client: IMMNotificationClient = DefaultDeviceNotice {
            changed: Arc::clone(changed),
        }
        .into();
        // SAFETY: The client is kept to be unregistered in `drop`
        unsafe { enumerator.RegisterEndpointNotificationCallback(&client) }?;
        Ok(Self { enumerator, client })
    }
}

impl Drop for DefaultDeviceWatch {
    fn drop(&mut self) {
        // SAFETY: Registered in `new`, on this thread. A failure only leaves
        // the client alive, whose event it keeps alive too.
        unsafe {
            self.enumerator
                .UnregisterEndpointNotificationCallback(&self.client)
        }
        .ok();
    }
}

#[implement(IMMNotificationClient)]
struct DefaultDeviceNotice {
    changed: Arc<Event>,
}

impl IMMNotificationClient_Impl for DefaultDeviceNotice_Impl {
    fn OnDefaultDeviceChanged(
        &self,
        flow: EDataFlow,
        role: ERole,
        _: &PCWSTR,
    ) -> windows::core::Result<()> {
        if flow == eRender && role == eConsole {
            self.changed.signal();
        }
        Ok(())
    }

    fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnDeviceAdded(&self, _: &PCWSTR) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnDeviceRemoved(&self, _: &PCWSTR) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnPropertyValueChanged(&self, _: &PCWSTR, _: &PROPERTYKEY) -> windows::core::Result<()> {
        Ok(())
    }
}

struct Event(HANDLE);

// SAFETY: Event handles may be waited on and signaled from any thread
unsafe impl Send for Event {}
// SAFETY: As above
unsafe impl Sync for Event {}

impl Event {
    fn new() -> Result<Self, WindowsError> {
        // SAFETY: No attributes or name to outlive the call
        unsafe { CreateEventW(None, false, false, PCWSTR::null()) }.map(Self)
    }

    fn signal(&self) {
        // SAFETY: The handle is open. It fails only for a bad handle.
        unsafe { SetEvent(self.0) }.ok();
    }
}

impl Drop for Event {
    fn drop(&mut self) {
        // SAFETY: The handle is open and owned here
        unsafe { CloseHandle(self.0) }.ok();
    }
}

struct Com {
    needs_uninitializing: bool,
}

impl Com {
    /// In the mode cpal initializes its threads in, as this may join it on one
    /// of those. WASAPI's first use on Windows 8 must not be from an MTA thread.
    fn new() -> Result<Self, Error> {
        // SAFETY: `drop` pairs it on this thread, as a `Com` stays on the
        // thread that made it
        let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if result.is_ok() || result == RPC_E_CHANGED_MODE {
            Ok(Self {
                needs_uninitializing: result.is_ok(),
            })
        } else {
            Err(Error::ExclusiveOutput(result.message()))
        }
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.needs_uninitializing {
            // SAFETY: Pairs with the successful `CoInitializeEx` in `new`
            unsafe { CoUninitialize() };
        }
    }
}

/// Keeps a short buffer from underrunning under load. Best effort: a stream
/// without it still plays.
struct ProAudioPriority(Option<HANDLE>);

impl ProAudioPriority {
    fn new() -> Self {
        let mut task_index = 0;
        // SAFETY: Writes `task_index`, which outlives the call
        let task = unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &raw mut task_index) };
        Self(task.ok())
    }
}

impl Drop for ProAudioPriority {
    fn drop(&mut self) {
        if let Some(task) = self.0 {
            // SAFETY: Reverts what `new` set on this same thread
            unsafe { AvRevertMmThreadCharacteristics(task) }.ok();
        }
    }
}
