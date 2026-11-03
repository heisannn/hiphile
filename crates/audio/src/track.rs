use std::io::{Read, Seek};
use std::sync::Arc;

use symphonia::core::formats::probe::Hint;
use symphonia::core::io::MediaSource;

/// Unblocks reads of a track's source from another thread.
pub(crate) type Cancel = Arc<dyn Fn() + Send + Sync>;

/// Encoded audio for the engine to decode.
pub struct Track {
    pub(crate) source: Box<dyn MediaSource>,
    pub(crate) hint: Hint,
    pub(crate) cancel: Option<Cancel>,
}

impl Track {
    /// A seekable source whose length is not reported to the decoder.
    ///
    /// Without a length the decoder does not look for metadata at the end of
    /// the file, so a source that is still downloading starts playing
    /// immediately instead of waiting for its last byte.
    pub fn new(reader: impl Read + Seek + Send + Sync + 'static) -> Self {
        Self {
            source: Box::new(Source {
                reader,
                byte_len: None,
            }),
            hint: Hint::new(),
            cancel: None,
        }
    }

    /// A source whose end is cheap to reach, such as a local file.
    pub fn with_known_len(reader: impl Read + Seek + Send + Sync + 'static, len: u64) -> Self {
        Self {
            source: Box::new(Source {
                reader,
                byte_len: Some(len),
            }),
            hint: Hint::new(),
            cancel: None,
        }
    }

    /// How to make a blocked read of this source return, for sources that
    /// wait on the network.
    ///
    /// The engine calls it when the track is replaced or stopped, or the
    /// engine is dropped, so those never wait for data that may not arrive.
    /// Afterwards the source should fail reads instead of blocking.
    pub fn cancel_with(mut self, cancel: impl Fn() + Send + Sync + 'static) -> Self {
        self.cancel = Some(Arc::new(cancel));
        self
    }

    /// MIME type such as `audio/flac`, to help the format probe.
    pub fn mime_type(mut self, mime_type: &str) -> Self {
        self.hint.mime_type(mime_type);
        self
    }

    /// File extension such as `flac`, to help the format probe.
    pub fn extension(mut self, extension: &str) -> Self {
        self.hint.with_extension(extension);
        self
    }
}

struct Source<R> {
    reader: R,
    byte_len: Option<u64>,
}

impl<R: Read> Read for Source<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.reader.read(buf)
    }
}

impl<R: Seek> Seek for Source<R> {
    fn seek(&mut self, position: std::io::SeekFrom) -> std::io::Result<u64> {
        self.reader.seek(position)
    }
}

impl<R: Read + Seek + Send + Sync> MediaSource for Source<R> {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        self.byte_len
    }
}
