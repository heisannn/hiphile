//! Each track decodes with symphonia on a thread of its own, as reading it may
//! wait on the network. One engine thread takes what that decodes, maps
//! channels, resamples only when the device runs at another rate than the
//! source, and writes into an `rtrb` ring buffer, without ever waiting for a
//! track's data. The cpal callback only copies out of that buffer: it never
//! allocates, locks, or blocks. Control goes through [`Engine`]'s methods,
//! outcomes come back as [`Event`]s, and the position is read from an atomic
//! frame counter instead of per-tick events.
//!
//! A track whose data comes too slowly to play on goes silent until a stretch
//! of it is ready, rather than sounding whatever little has arrived.
//!
//! A track set to follow the current one is written to the same ring buffer
//! when the device does not have to change for it, so no sample is lost
//! between them. Otherwise the stream reopens at the track's rate in between.
//!
//! On PCM, the silence around a track goes on its decoding thread, which still
//! reports positions in the file. A crossfade writes both tracks to the same ring
//! buffer too, mixed after each one's rate conversion, from the moment the first
//! has only the length of the fade left.
//!
//! Also on PCM, a pause or stop can fall to silence and playing again rise from
//! it. The callback does this to what it takes from the ring buffer, and the
//! stream stops once it hears nothing.
//!
//! With DoP settings, the converter turns the track into DSD words on the
//! engine thread, the same ring buffer carries those, and the callback makes
//! them DoP frames at full level. The engine then holds the device for itself,
//! as it does for PCM in exclusive mode: through hog mode on macOS, and on
//! Windows through a WASAPI stream of its own, as cpal's are all shared.

mod convert;
mod decode;
mod device;
mod engine;
mod error;
mod exclusive_device;
mod feed;
mod from_config;
mod mix;
mod output;
mod play_fade;
mod stream;
#[cfg(test)]
mod test_wav;
mod track;

pub use device::{OutputDevice, output_devices};
pub use dsd::{DopRate, Settings as DsdSettings};
pub use engine::{Engine, Event, EventKind, LoadId, PlaybackFormat, Transitions};
pub use error::Error;
pub use output::{
    OutputSettings, R8brainPhase, R8brainSettings, Resampling, ResamplingWindow, SoxrSettings,
};
pub use track::Track;

/// Whether this platform's audio can carry DoP at all. Its shared modes mix
/// and resample elsewhere, which would turn DoP into noise.
pub const CAN_PLAY_DOP: bool = cfg!(target_os = "macos");

/// Whether a device that is not held runs at the rate a stream asks for. Windows
/// converts such a stream to its mix rate, so raising the rate there only adds a
/// second conversion.
pub(crate) const SHARED_STREAM_SETS_RATE: bool = !cfg!(target_os = "windows");

/// Whether this platform can hold a device for one app while playing PCM.
pub const CAN_PLAY_EXCLUSIVE: bool = cfg!(any(target_os = "macos", target_os = "windows"));
