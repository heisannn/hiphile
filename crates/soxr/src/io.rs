use std::fmt::Debug;

use crate::Error;

mod sealed {
    pub trait Sealed {}
}

/// A type of sample: `f32`, `f64`, `i32` or `i16`. Full scale is 1 for
/// floating point, 2^31 for `i32` and 2^15 for `i16`, so a resampler from `i16`
/// to `f32` makes 16384 into 0.5.
pub trait Sample: sealed::Sealed + Copy + Default + Debug + Send + Sync + 'static {
    #[doc(hidden)]
    const FULL_SCALE: f64;
    /// Takes dither, when [`Dither::Tpdf`] is chosen
    #[doc(hidden)]
    const TAKES_DITHER: bool;
    #[doc(hidden)]
    fn to_f64(self) -> f64;
    /// From a value in units of this type, rounded to the nearest with ties away
    /// from zero and held to the type's range, adding to `clips` when it is not
    /// a floating-point type and the rounded value lies outside. NaN becomes 0.
    #[doc(hidden)]
    fn from_f64(value: f64, clips: &mut u64) -> Self;
}

impl sealed::Sealed for f32 {}
impl sealed::Sealed for f64 {}
impl sealed::Sealed for i32 {}
impl sealed::Sealed for i16 {}

impl Sample for f32 {
    const FULL_SCALE: f64 = 1.0;
    const TAKES_DITHER: bool = false;

    fn to_f64(self) -> f64 {
        f64::from(self)
    }

    fn from_f64(value: f64, _: &mut u64) -> Self {
        value as f32
    }
}

impl Sample for f64 {
    const FULL_SCALE: f64 = 1.0;
    const TAKES_DITHER: bool = false;

    fn to_f64(self) -> f64 {
        self
    }

    fn from_f64(value: f64, _: &mut u64) -> Self {
        value
    }
}

impl Sample for i32 {
    const FULL_SCALE: f64 = 2_147_483_648.0;
    const TAKES_DITHER: bool = false;

    fn to_f64(self) -> f64 {
        f64::from(self)
    }

    fn from_f64(value: f64, clips: &mut u64) -> Self {
        let rounded = value.round();
        if rounded > f64::from(i32::MAX) {
            *clips += 1;
            i32::MAX
        } else if rounded < f64::from(i32::MIN) {
            *clips += 1;
            i32::MIN
        } else {
            // NaN casts to 0
            rounded as i32
        }
    }
}

impl Sample for i16 {
    const FULL_SCALE: f64 = 32_768.0;
    const TAKES_DITHER: bool = true;

    fn to_f64(self) -> f64 {
        f64::from(self)
    }

    fn from_f64(value: f64, clips: &mut u64) -> Self {
        let rounded = value.round();
        if rounded > f64::from(i16::MAX) {
            *clips += 1;
            i16::MAX
        } else if rounded < f64::from(i16::MIN) {
            *clips += 1;
            i16::MIN
        } else {
            rounded as i16
        }
    }
}

/// Noise added before rounding to `i16`. Other output types take none.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dither {
    /// Triangular, peaking at ±1 LSB. With no `seed`, each resampler draws its
    /// own. [`crate::Resampler::clear`] starts the same noise again.
    Tpdf {
        seed: Option<u64>,
    },
    None,
}

impl Default for Dither {
    fn default() -> Self {
        Self::Tpdf { seed: None }
    }
}

/// How samples go in and come out. Their types are those of the
/// [`crate::Resampler`].
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct IoSpec {
    /// Gain on top of the change of full scale between the types
    pub scale: f64,
    pub dither: Dither,
}

impl IoSpec {
    pub const fn new() -> Self {
        Self {
            scale: 1.0,
            dither: Dither::Tpdf { seed: None },
        }
    }

    pub const fn with_scale(self, scale: f64) -> Self {
        Self { scale, ..self }
    }

    pub const fn with_dither(self, dither: Dither) -> Self {
        Self { dither, ..self }
    }
}

impl Default for IoSpec {
    fn default() -> Self {
        Self::new()
    }
}

/// Samples for every channel: in frames of one sample per channel, or a
/// buffer per channel. Lengths count frames.
#[derive(Clone, Copy, Debug)]
pub enum Input<'a, T> {
    Interleaved(&'a [T]),
    Split(&'a [&'a [T]]),
}

impl<'a, T> From<&'a [T]> for Input<'a, T> {
    fn from(samples: &'a [T]) -> Self {
        Self::Interleaved(samples)
    }
}

impl<'a, T> From<&'a Vec<T>> for Input<'a, T> {
    fn from(samples: &'a Vec<T>) -> Self {
        Self::Interleaved(samples)
    }
}

impl<'a, T> From<&'a [&'a [T]]> for Input<'a, T> {
    fn from(buffers: &'a [&'a [T]]) -> Self {
        Self::Split(buffers)
    }
}

/// Room for samples of every channel, as [`Input`] holds them
#[derive(Debug)]
pub enum Output<'a, 'b, T> {
    Interleaved(&'a mut [T]),
    Split(&'a mut [&'b mut [T]]),
}

impl<'a, T> From<&'a mut [T]> for Output<'a, '_, T> {
    fn from(samples: &'a mut [T]) -> Self {
        Self::Interleaved(samples)
    }
}

impl<'a, T> From<&'a mut Vec<T>> for Output<'a, '_, T> {
    fn from(samples: &'a mut Vec<T>) -> Self {
        Self::Interleaved(samples)
    }
}

impl<'a, 'b, T> From<&'a mut [&'b mut [T]]> for Output<'a, 'b, T> {
    fn from(buffers: &'a mut [&'b mut [T]]) -> Self {
        Self::Split(buffers)
    }
}

impl<T> Input<'_, T> {
    pub(crate) fn frames(&self, channels: usize) -> Result<usize, Error> {
        match self {
            Self::Interleaved(samples) if samples.len() % channels == 0 => {
                Ok(samples.len() / channels)
            }
            Self::Split(buffers) if buffers.len() == channels => equal_len(buffers.iter().copied()),
            _ => Err(Error::BufferLayout),
        }
    }

    /// Of `channel`, the frames from `start` on
    pub(crate) fn channel(&self, channel: usize, channels: usize, start: usize) -> Channel<'_, T> {
        match self {
            Self::Interleaved(samples) => Channel {
                samples: &samples[(start * channels + channel).min(samples.len())..],
                stride: channels,
            },
            Self::Split(buffers) => Channel {
                samples: &buffers[channel][start..],
                stride: 1,
            },
        }
    }
}

impl<T: Copy> Input<'_, T> {
    /// When it is interleaved stereo, appends its first `frames` frames to
    /// `left` and `right` through `convert`, in one loop that the compiler
    /// vectorizes, and returns whether it did
    pub(crate) fn split_stereo<U: Copy + Default>(
        &self,
        frames: usize,
        left: &mut Vec<U>,
        right: &mut Vec<U>,
        convert: impl Fn(T) -> U,
    ) -> bool {
        let Self::Interleaved(samples) = self else {
            return false;
        };
        let (left_base, right_base) = (left.len(), right.len());
        left.resize(left_base + frames, U::default());
        right.resize(right_base + frames, U::default());
        let slots = left[left_base..].iter_mut().zip(&mut right[right_base..]);
        for ((left, right), frame) in slots.zip(samples[..frames * 2].as_chunks::<2>().0) {
            *left = convert(frame[0]);
            *right = convert(frame[1]);
        }
        true
    }
}

impl<T> Output<'_, '_, T> {
    pub(crate) fn frames(&self, channels: usize) -> Result<usize, Error> {
        match self {
            Self::Interleaved(samples) if samples.len() % channels == 0 => {
                Ok(samples.len() / channels)
            }
            Self::Split(buffers) if buffers.len() == channels => {
                equal_len(buffers.iter().map(|buffer| &**buffer))
            }
            _ => Err(Error::BufferLayout),
        }
    }

    /// Of `channel`, the slots from frame `start` on: the first of each
    /// chunk of the returned length
    pub(crate) fn channel(
        &mut self,
        channel: usize,
        channels: usize,
        start: usize,
    ) -> (&mut [T], usize) {
        match self {
            Self::Interleaved(buffer) => {
                let from = (start * channels + channel).min(buffer.len());
                (&mut buffer[from..], channels)
            }
            Self::Split(buffers) => (&mut buffers[channel][start..], 1),
        }
    }
}

/// One channel of an [`Input`]
pub(crate) struct Channel<'a, T> {
    samples: &'a [T],
    stride: usize,
}

impl<T: Copy> Channel<'_, T> {
    /// Appends its first `frames` samples to `out` through `convert`
    pub(crate) fn extend<U: Copy + Default>(
        &self,
        frames: usize,
        out: &mut Vec<U>,
        convert: impl Fn(T) -> U,
    ) {
        let base = out.len();
        out.resize(base + frames, U::default());
        let slots = &mut out[base..];
        if self.stride == 1 {
            for (slot, sample) in slots.iter_mut().zip(&self.samples[..frames]) {
                *slot = convert(*sample);
            }
        } else {
            for (slot, frame) in slots.iter_mut().zip(self.samples.chunks(self.stride)) {
                *slot = convert(frame[0]);
            }
        }
    }
}

fn equal_len<'a, T: 'a>(mut buffers: impl Iterator<Item = &'a [T]>) -> Result<usize, Error> {
    let first = buffers.next().map_or(0, <[T]>::len);
    if buffers.all(|buffer| buffer.len() == first) {
        Ok(first)
    } else {
        Err(Error::BufferLayout)
    }
}
