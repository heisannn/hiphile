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

mod convert;
mod decode;
mod device;
mod engine;
mod error;
mod feed;
mod output;
#[cfg(test)]
mod test_wav;
mod track;

pub use device::{OutputDevice, output_devices};
pub use engine::{Engine, Event, EventKind, LoadId, PlaybackFormat};
pub use error::Error;
pub use output::OutputSettings;
pub use track::Track;
