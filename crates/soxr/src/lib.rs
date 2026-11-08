//! Sample rate converter with the functions of the SoX Resampler library
//! (libsoxr), written from descriptions and observations of its behaviour,
//! without its code.
//!
//! A [`Resampler`] converts any number of channels between two rates at any
//! constant ratio, or at a ratio that changes, with the quality chosen by a
//! [`QualitySpec`]. Its output starts at the input's first sample, with the
//! filters' latency removed, and ends after `round(input frames / ratio)`
//! frames.

pub mod bits;
mod chain;
mod clock;
mod design;
mod dither;
mod error;
mod fft_stage;
mod io;
mod lanes;
mod phase;
mod poly;
mod quality;
mod real;
mod resampler;
mod runtime;
mod simd;
mod source;
mod variable;

pub use error::Error;
pub use io::{Dither, Input, IoSpec, Output, Sample};
pub use quality::{Engine, Phase, Quality, QualitySpec, Recipe, Rolloff};
pub use resampler::{Builder, Progress, Resampler};
pub use runtime::{CoefInterpolation, RuntimeSpec};
pub use source::{Source, SourceError};
