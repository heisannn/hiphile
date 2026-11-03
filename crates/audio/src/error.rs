#[derive(Clone, Debug, thiserror::Error)]
pub enum Error {
    #[error("no audio output device is available")]
    NoOutputDevice,
    #[error("the output device does not accept 32-bit float audio")]
    UnsupportedOutputFormat,
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

impl From<cpal::Error> for Error {
    fn from(error: cpal::Error) -> Self {
        Self::Output(error.to_string())
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
