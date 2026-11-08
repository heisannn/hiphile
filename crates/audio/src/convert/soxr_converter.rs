use soxr::Resampler;

use crate::Error;
use crate::output::SoxrSettings;

/// Output room past what the input completes, for output that a filter's FFT block lets out at
/// once. Less still gets all of it out, in more calls.
const SPARE_FRAMES: usize = 4096;

/// One resampler for every channel, over interleaved samples. The output starts with the input's
/// first frame, and soxr ends it at `floor(input frames / io ratio + 0.5)` frames.
pub(super) struct SoxrConverter {
    resampler: Resampler<f32, f32>,
    input: Vec<f32>,
    frames_in: u64,
    frames_out: u64,
}

impl SoxrConverter {
    pub(super) fn new(
        from: u32,
        to: u32,
        channels: usize,
        settings: SoxrSettings,
    ) -> Result<Self, Error> {
        let resampler = Resampler::builder(f64::from(from), f64::from(to), channels)
            .quality(settings.quality)
            .runtime(settings.runtime)
            .build()
            .map_err(to_error)?;
        Ok(Self {
            resampler,
            input: Vec::new(),
            frames_in: 0,
            frames_out: 0,
        })
    }

    /// A sample that is not a finite number counts as silence: it would spread
    /// through a whole FFT block.
    pub(super) fn push(&mut self, samples: &[f32], out: &mut Vec<f32>) -> Result<(), Error> {
        self.input.clear();
        self.input.extend(
            samples
                .iter()
                .map(|sample| if sample.is_finite() { *sample } else { 0.0 }),
        );
        let channels = self.resampler.channels();
        self.frames_in += (samples.len() / channels) as u64;
        let mut rest = &self.input[..];
        loop {
            let frames = rest.len() / channels;
            let room = (frames as f64 / self.resampler.io_ratio()).ceil() as usize + SPARE_FRAMES;
            let start = out.len();
            out.resize(start + room * channels, 0.0);
            let progress = self
                .resampler
                .resample(rest, &mut out[start..])
                .map_err(to_error)?;
            out.truncate(start + progress.output_frames * channels);
            self.frames_out += progress.output_frames as u64;
            rest = &rest[progress.input_frames * channels..];
            if rest.is_empty() && progress.output_frames < room {
                return Ok(());
            }
        }
    }

    pub(super) fn undelivered_frames(&self) -> u64 {
        self.expected_frames().saturating_sub(self.frames_out)
    }

    pub(super) fn finish(&mut self, out: &mut Vec<f32>) -> Result<(), Error> {
        let channels = self.resampler.channels();
        loop {
            let room = self.undelivered_frames() as usize;
            let start = out.len();
            out.resize(start + room * channels, 0.0);
            let written = self.resampler.finish(&mut out[start..]).map_err(to_error)?;
            out.truncate(start + written * channels);
            self.frames_out += written as u64;
            if written == 0 {
                return Ok(());
            }
        }
    }

    /// As soxr counts it
    fn expected_frames(&self) -> u64 {
        (self.frames_in as f64 / self.resampler.io_ratio() + 0.5).floor() as u64
    }
}

fn to_error(error: soxr::Error) -> Error {
    Error::Resample(error.to_string())
}
