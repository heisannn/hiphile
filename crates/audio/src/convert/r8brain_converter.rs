use r8brain::Resampler;

use crate::Error;
use crate::output::R8brainSettings;

/// A resampler per channel, over interleaved samples. The output starts with
/// the input's first frame, and its length follows the ratio.
pub(super) struct R8brainConverter {
    resamplers: Vec<Resampler>,
    ratio: f64,
    /// As many samples as the resamplers plan to take at a time, of all channels
    block_len: usize,
    channel_input: Vec<f64>,
    frames_in: u64,
    frames_out: u64,
}

impl R8brainConverter {
    pub(super) fn new(
        from: u32,
        to: u32,
        channels: usize,
        settings: R8brainSettings,
    ) -> Result<Self, Error> {
        let to_error = |error: r8brain::Error| Error::Resample(error.to_string());
        settings.cache_limits.apply().map_err(to_error)?;
        let resamplers = (0..channels)
            .map(|_| Resampler::new(f64::from(from), f64::from(to), settings.options))
            .collect::<Result<_, _>>()
            .map_err(to_error)?;
        Ok(Self {
            resamplers,
            ratio: f64::from(to) / f64::from(from),
            block_len: settings.options.max_input_len * channels,
            channel_input: Vec::new(),
            frames_in: 0,
            frames_out: 0,
        })
    }

    pub(super) fn push(&mut self, samples: &[f32], out: &mut Vec<f32>) {
        self.frames_in += (samples.len() / self.resamplers.len()) as u64;
        for block in samples.chunks(self.block_len) {
            self.convert(block, out);
        }
    }

    pub(super) fn undelivered_frames(&self) -> u64 {
        self.expected_frames().saturating_sub(self.frames_out)
    }

    pub(super) fn finish(&mut self, out: &mut Vec<f32>) {
        let expected = self.expected_frames();
        let channels = self.resamplers.len();
        let needed = self.resamplers[0].input_required_for_output(expected as usize) as u64;
        let tail_frames = needed.saturating_sub(self.frames_in).max(1) as usize;
        let silence = vec![0.0; self.block_len.min(tail_frames * channels)];
        while self.frames_out < expected {
            self.convert(&silence, out);
        }
        let surplus = (self.frames_out - expected) as usize * channels;
        out.truncate(out.len() - surplus);
        self.frames_out = expected;
    }

    fn expected_frames(&self) -> u64 {
        (self.frames_in as f64 * self.ratio).round() as u64
    }

    /// A sample that is not a finite number counts as silence: it would spread
    /// through a whole FFT block.
    fn convert(&mut self, samples: &[f32], out: &mut Vec<f32>) {
        let channels = self.resamplers.len();
        let start = out.len();
        for (channel, resampler) in self.resamplers.iter_mut().enumerate() {
            self.channel_input.clear();
            self.channel_input
                .extend(samples.chunks_exact(channels).map(|frame| {
                    let sample = frame[channel];
                    if sample.is_finite() {
                        f64::from(sample)
                    } else {
                        0.0
                    }
                }));
            let resampled = resampler.process(&self.channel_input);
            // All channels are in step, so the first says how long they all are
            if channel == 0 {
                out.resize(start + resampled.len() * channels, 0.0);
            }
            for (frame, sample) in out[start..].chunks_exact_mut(channels).zip(resampled) {
                frame[channel] = *sample as f32;
            }
        }
        self.frames_out += ((out.len() - start) / channels) as u64;
    }
}
