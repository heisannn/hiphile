//! Playback engine.
//!
//! One engine thread decodes with symphonia, maps channels, resamples only
//! when the device runs at another rate than the source, and writes into an
//! `rtrb` ring buffer. The cpal callback only copies out of that buffer: it never
//! allocates, locks, or blocks. Control goes through [`Engine`]'s methods,
//! outcomes come back as [`Event`]s, and the position is read from an atomic
//! frame counter instead of per-tick events.

mod convert;
mod decode;
mod device;
mod engine;
mod error;
mod output;
#[cfg(test)]
mod test_wav;
mod track;

pub use device::{OutputDevice, output_devices};
pub use engine::{Engine, Event, EventKind, LoadId, PlaybackFormat};
pub use error::Error;
pub use output::OutputSettings;
pub use track::Track;
