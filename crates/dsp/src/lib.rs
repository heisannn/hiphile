//! Crossfeed and balance on interleaved `f32` frames. The engine runs them at
//! a song's own rate, before the song is resampled or encoded as DSD.
//!
//! A [`Processor`] never allocates, locks or blocks, and moves between
//! settings without a click. With neutral [`Settings`] it leaves the samples
//! untouched, bit for bit.

mod crossfeed;
mod processor;
mod settings;

pub use processor::Processor;
pub use settings::{Balance, Crossfeed, Settings};
