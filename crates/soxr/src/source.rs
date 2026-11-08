use std::num::NonZeroUsize;

use crate::Input;

/// Where [`crate::Resampler::output`] pulls its input from
pub trait Source<T> {
    /// Up to `frames` frames of input, lent until the resampler has taken
    /// them. None ends the input. More frames than asked for, or buffers that
    /// do not match the resampler's channels, count as a failure.
    fn pull(&mut self, frames: usize) -> Result<Input<'_, T>, SourceError>;

    /// The most frames to ask for at once. With none, a call asks for
    /// `ceil(room × io_ratio)`.
    fn most_frames(&self) -> Option<NonZeroUsize> {
        None
    }
}

/// A [`Source`] that could not give input. It stops the resampler with
/// [`crate::Error::InputFailure`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, thiserror::Error)]
#[error("the source could not give input")]
pub struct SourceError;
