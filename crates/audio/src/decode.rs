use std::time::Duration;

use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as Symphonia;
use symphonia::core::formats::{FormatOptions, FormatReader, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

use crate::output::Spec;
use crate::{Error, Track};

/// Turns a [`Track`] into interleaved `f32` samples at the source rate.
pub(crate) struct Decoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    spec: Spec,
    duration: Option<Duration>,
    /// Samples decoded while discovering the spec, returned first.
    pending: Vec<f32>,
}

impl Decoder {
    pub(crate) fn open(track: Track) -> Result<Self, Error> {
        let stream = MediaSourceStream::new(track.source, Default::default());
        let format = symphonia::default::get_probe().probe(
            &track.hint,
            stream,
            FormatOptions::default(),
            MetadataOptions::default(),
        )?;
        let audio = format
            .default_track(TrackType::Audio)
            .ok_or_else(|| Error::UnsupportedFormat("no audio track".to_owned()))?;
        let params = audio
            .codec_params
            .as_ref()
            .and_then(|params| params.audio())
            .ok_or_else(|| Error::UnsupportedFormat("no audio codec parameters".to_owned()))?;
        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(params, &AudioDecoderOptions::default())?;
        let declared = params
            .sample_rate
            .zip(params.channels.as_ref().map(|channels| channels.count()));
        let track_id = audio.id;
        let num_frames = audio.num_frames;

        let mut decoder = Self {
            format,
            decoder,
            track_id,
            spec: Spec {
                rate: 0,
                channels: 0,
            },
            duration: None,
            pending: Vec::new(),
        };
        decoder.spec = match declared {
            Some((rate, channels)) => Spec {
                rate,
                channels: u16::try_from(channels).unwrap_or(u16::MAX),
            },
            // Some streams only reveal their layout in the first packet.
            None => {
                let mut pending = Vec::new();
                let spec = decoder
                    .decode_packet(&mut pending)?
                    .ok_or_else(|| Error::Decode("the file contains no audio".to_owned()))?;
                decoder.pending = pending;
                spec
            }
        };
        if decoder.spec.rate == 0 || decoder.spec.channels == 0 {
            return Err(Error::UnsupportedFormat("unknown sample layout".to_owned()));
        }
        decoder.duration = num_frames
            .map(|frames| Duration::from_secs_f64(frames as f64 / f64::from(decoder.spec.rate)));
        Ok(decoder)
    }

    pub(crate) fn spec(&self) -> Spec {
        self.spec
    }

    pub(crate) fn duration(&self) -> Option<Duration> {
        self.duration
    }

    /// Appends the next packet's samples. Returns `false` at the end.
    pub(crate) fn decode_into(&mut self, out: &mut Vec<f32>) -> Result<bool, Error> {
        if !self.pending.is_empty() {
            out.append(&mut self.pending);
            return Ok(true);
        }
        let Some(spec) = self.decode_packet(out)? else {
            return Ok(false);
        };
        if spec != self.spec {
            return Err(Error::UnsupportedFormat(
                "the sample layout changes mid-stream".to_owned(),
            ));
        }
        Ok(true)
    }

    fn decode_packet(&mut self, out: &mut Vec<f32>) -> Result<Option<Spec>, Error> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) => return Ok(None),
                // A truncated final packet ends the stream rather than failing it.
                Err(Symphonia::IoError(error))
                    if error.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    return Ok(None);
                }
                Err(error) => return Err(error.into()),
            };
            if packet.track_id != self.track_id {
                continue;
            }
            match self.decoder.decode(&packet) {
                Ok(buffer) => {
                    let spec = Spec {
                        rate: buffer.spec().rate(),
                        channels: u16::try_from(buffer.spec().channels().count())
                            .unwrap_or(u16::MAX),
                    };
                    let start = out.len();
                    out.resize(start + buffer.samples_interleaved(), 0.0);
                    buffer.copy_to_slice_interleaved(&mut out[start..]);
                    return Ok(Some(spec));
                }
                // A corrupt packet is skipped; the next one may decode.
                Err(Symphonia::DecodeError(_)) => continue,
                Err(error) => return Err(error.into()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::test_wav::wav;

    fn decode_all(decoder: &mut Decoder) -> Vec<f32> {
        let mut samples = Vec::new();
        while decoder.decode_into(&mut samples).unwrap() {}
        samples
    }

    #[test]
    fn decodes_wav_with_its_spec_and_duration() {
        let bytes = wav(48_000, 2, 24_000, i16::MAX / 2);
        let mut decoder = Decoder::open(Track::new(Cursor::new(bytes)).extension("wav")).unwrap();
        assert_eq!(
            decoder.spec(),
            Spec {
                rate: 48_000,
                channels: 2
            }
        );
        assert_eq!(decoder.duration(), Some(Duration::from_millis(500)));
        let samples = decode_all(&mut decoder);
        assert_eq!(samples.len(), 48_000);
        assert!((samples[0] - 0.5).abs() < 0.001);
    }

    #[test]
    fn probes_without_a_hint() {
        let bytes = wav(44_100, 1, 100, 0);
        assert!(Decoder::open(Track::new(Cursor::new(bytes))).is_ok());
    }

    #[test]
    fn rejects_data_that_is_not_audio() {
        let track = Track::new(Cursor::new(b"<html>not audio</html>".to_vec()));
        assert!(Decoder::open(track).is_err());
    }
}
