#[derive(Clone, Debug, thiserror::Error)]
pub enum Error {
    #[error("no audio output device is available")]
    NoOutputDevice,
    #[error("the chosen output device is not connected")]
    OutputDeviceUnavailable,
    #[error("the output device does not accept 32-bit float audio")]
    UnsupportedOutputFormat,
    #[error("the output device cannot run at the rate this DSD rate needs")]
    UnsupportedDopRate,
    #[error("DSD needs the output device for itself, and process {0} is holding it")]
    DeviceHeld(i32),
    #[error("could not take the output device for DSD: {0}")]
    ExclusiveOutput(String),
    #[error("the DSD modulator became unstable: lower Noise gain or Input level")]
    DsdUnstable,
    #[error("audio output failed: {0}")]
    Output(String),
    #[error("the file format is not supported: {0}")]
    UnsupportedFormat(String),
    #[error("the file could not be decoded: {0}")]
    Decode(String),
    #[error("could not read the file: {0}")]
    Read(String),
    #[error("sample rate conversion failed: {0}")]
    Resample(String),
}

impl Error {
    /// Whether the track's bytes are at fault, a damaged or unsupported
    /// file, rather than the output device or the resampler.
    pub fn is_track_error(&self) -> bool {
        matches!(
            self,
            Self::UnsupportedFormat(_) | Self::Decode(_) | Self::Read(_)
        )
    }
}

impl From<cpal::Error> for Error {
    fn from(error: cpal::Error) -> Self {
        Self::Output(error.to_string())
    }
}

impl From<dsd::Error> for Error {
    fn from(error: dsd::Error) -> Self {
        match error {
            dsd::Error::Resample(message) => Self::Resample(message),
            dsd::Error::Unstable => Self::DsdUnstable,
        }
    }
}

impl From<symphonia::core::errors::Error> for Error {
    fn from(error: symphonia::core::errors::Error) -> Self {
        use symphonia::core::errors::Error as Symphonia;
        match error {
            Symphonia::IoError(error) => Self::Read(error.to_string()),
            Symphonia::Unsupported(what) => Self::UnsupportedFormat(what.to_owned()),
            error => Self::Decode(error.to_string()),
        }
    }
}
