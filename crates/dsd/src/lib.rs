//! Turns stereo PCM into the 16-bit DSD words that DoP frames carry. A track
//! goes through an oversampler to the DSD bit rate, a look-ahead limiter and
//! one 1-bit modulator per channel. The markers that make those words DoP are
//! left to whoever hands them to the device, as their alternation has to
//! survive anything that drops words on the way there.

mod encoder;
mod limiter;
mod modulator;
mod noise_transfer;
mod oversampler;
mod settings;

pub use encoder::Encoder;
pub use settings::{
    DopRate, Filter, Modulation, NoiseBand, Order, ResamplingWindow, Settings, SincLength,
};

/// The 16 DSD bits that mean silence: an even pattern a DAC decodes to its
/// mid level.
pub const SILENCE_WORD: u16 = 0x6969;

/// The markers that alternate frame by frame, the same on every channel of a
/// frame.
pub const DOP_MARKERS: [u8; 2] = [0x05, 0xFA];

#[derive(Clone, Debug, thiserror::Error)]
pub enum Error {
    #[error("sample rate conversion failed: {0}")]
    Resample(String),
    #[error("the DSD modulator became unstable")]
    Unstable,
}
