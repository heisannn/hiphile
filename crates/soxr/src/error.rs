/// What went wrong. The messages of the variants that libsoxr also reports are
/// its own, word for word, so that logs read the same.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Upsampling with a stop-band that begins further above the Nyquist
    /// frequency than the pass-band ends below it
    #[error("imaging greater than rolloff")]
    ImagingGreaterThanRolloff,
    /// The stop-band begins less than 0.002 or more than 0.5 above the
    /// pass-band
    #[error("transition bandwidth not in [0.2,50] % of nyquist")]
    TransitionBandwidth,
    /// The pass-band ends under 0.5 or the stop-band begins over 1.5
    #[error("transition band not within [50,150] % of nyquist")]
    TransitionBand,
    #[error("precision not in [15,33] bits")]
    Precision,
    #[error("phase response not in [0=min-phase,100=max-phase] %")]
    PhaseResponse,
    /// A rate or ratio that is not positive and finite
    #[error("I/O ratio out-of-range")]
    IoRatioOutOfRange,
    #[error("invalid # of channels")]
    InvalidChannels,
    /// A new ratio for a resampler that keeps one ratio
    #[error("varying O/I ratio is not supported with this quality level")]
    VaryingRatio,
    /// Kept by the resampler until [`crate::Resampler::clear`]
    #[error("input function reported failure")]
    InputFailure,
    #[error("input after the end of the input")]
    InputAfterEnd,
    /// A buffer for another number of channels, split buffers of unequal
    /// lengths, or an interleaved buffer that ends within a frame
    #[error("buffers do not match the channels")]
    BufferLayout,
    #[error("scale not finite")]
    Scale,
}
