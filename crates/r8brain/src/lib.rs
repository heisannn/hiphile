//! Sample rate converter ported from
//! [r8brain-free-src](https://github.com/avaneev/r8brain-free-src), designed by
//! Aleksey Vaneev of Voxengo.
//!
//! A [`Resampler`] chains the same steps as the original. Steep filtering runs
//! as overlap-save FFT convolution at twice the lower of the two rates, and a
//! bank of short sinc filters then interpolates to the target rate. Ratios of
//! powers of two and three skip the interpolation, and larger ones add cheap
//! half-band steps.
//!
//! The FFT comes from `realfft` rather than the Ooura code the original
//! bundles, so results agree with the original up to rounding, not bit for bit.
//!
//! Each [`Resampler`] holds the state of one channel. Filters and tables are
//! shared between resamplers and kept for later ones, so creating another for
//! the same rates and options takes little time.

// Constants stay as the original has them, to compare and to search for
#![allow(clippy::excessive_precision)]

mod block_convolver;
mod cache;
mod fft;
mod filter_bank;
mod fir_filter;
mod half_band;
mod interpolator;
mod math;
mod processor;
mod resampler;
mod sinc;

pub use resampler::{CacheLimits, Error, Options, Phase, Resampler};
