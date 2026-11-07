use std::time::Duration;

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Indexing, Resampler as _};

use crate::Error;
use crate::output::{OutputSettings, Resampling, Spec};

const RESAMPLE_CHUNK: usize = 1024;

pub(crate) struct Converter {
    source: Spec,
    output: Spec,
    stage: Stage,
    mapped: Vec<f32>,
    words: Vec<u16>,
}

enum Stage {
    Pcm {
        resampling: Resampling,
        resampler: Option<Box<RateConverter>>,
    },
    /// Its samples are DSD words as whole numbers, which `f32` holds exactly.
    Dop(Box<dsd::Encoder>),
}

impl Converter {
    pub(crate) fn new(source: Spec, output: Spec, resampling: Resampling) -> Result<Self, Error> {
        let resampler = Self::resampler(source, output, resampling)?;
        Ok(Self::around(
            source,
            output,
            Stage::Pcm {
                resampling,
                resampler,
            },
        ))
    }

    /// As `settings` say, for `source` on a stream of `output`.
    pub(crate) fn for_settings(
        source: Spec,
        output: Spec,
        settings: &OutputSettings,
    ) -> Result<Self, Error> {
        match settings.dop() {
            None => Self::new(source, output, settings.resampling()),
            Some(dop) => {
                let encoder = dsd::Encoder::new(source.rate, output.rate, dop)?;
                Ok(Self::around(source, output, Stage::Dop(Box::new(encoder))))
            }
        }
    }

    /// For the track that comes on after this one, on the same stream.
    pub(crate) fn following(&self, source: Spec, settings: &OutputSettings) -> Result<Self, Error> {
        match &self.stage {
            Stage::Pcm { .. } => Self::for_settings(source, self.output, settings),
            Stage::Dop(encoder) => {
                let encoder = encoder.for_next_track(source.rate)?;
                Ok(Self::around(
                    source,
                    self.output,
                    Stage::Dop(Box::new(encoder)),
                ))
            }
        }
    }

    fn around(source: Spec, output: Spec, stage: Stage) -> Self {
        Self {
            source,
            output,
            stage,
            mapped: Vec::new(),
            words: Vec::new(),
        }
    }

    fn resampler(
        source: Spec,
        output: Spec,
        resampling: Resampling,
    ) -> Result<Option<Box<RateConverter>>, Error> {
        (source.rate != output.rate)
            .then(|| {
                RateConverter::new(
                    source.rate,
                    output.rate,
                    usize::from(output.channels),
                    resampling,
                )
                .map(Box::new)
            })
            .transpose()
    }

    pub(crate) fn reset(&mut self) -> Result<(), Error> {
        match &mut self.stage {
            Stage::Pcm {
                resampling,
                resampler,
            } => *resampler = Self::resampler(self.source, self.output, *resampling)?,
            Stage::Dop(encoder) => encoder.reset()?,
        }
        Ok(())
    }

    /// Raises DoP output from silence, starting with the next input.
    pub(crate) fn fade_in(&mut self, length: Duration) {
        if let Stage::Dop(encoder) = &mut self.stage {
            encoder.fade_in(length);
        }
    }

    pub(crate) fn is_fading(&self) -> bool {
        matches!(&self.stage, Stage::Dop(encoder) if encoder.is_fading())
    }

    pub(crate) fn process(&mut self, input: &[f32], out: &mut Vec<f32>) -> Result<(), Error> {
        let from = usize::from(self.source.channels);
        let to = usize::from(self.output.channels);
        match &mut self.stage {
            Stage::Pcm {
                resampler: None, ..
            } => {
                map_channels(input, from, to, out);
                Ok(())
            }
            Stage::Pcm {
                resampler: Some(resampler),
                ..
            } => {
                self.mapped.clear();
                map_channels(input, from, to, &mut self.mapped);
                resampler.push(&self.mapped, out)
            }
            Stage::Dop(encoder) => {
                self.mapped.clear();
                map_channels(input, from, to, &mut self.mapped);
                self.words.clear();
                encoder.push(&self.mapped, &mut self.words)?;
                out.extend(self.words.iter().copied().map(f32::from));
                Ok(())
            }
        }
    }

    /// Frames that the input so far comes to, still inside the resampler.
    pub(crate) fn undelivered_frames(&self) -> u64 {
        match &self.stage {
            Stage::Pcm { resampler, .. } => resampler
                .as_ref()
                .map_or(0, |resampler| resampler.undelivered_frames()),
            Stage::Dop(encoder) => encoder.undelivered_frames(),
        }
    }

    /// Appends whatever the resampler still holds once the input has ended.
    pub(crate) fn finish(&mut self, out: &mut Vec<f32>) -> Result<(), Error> {
        match &mut self.stage {
            Stage::Pcm { resampler, .. } => resampler
                .as_mut()
                .map_or(Ok(()), |resampler| resampler.finish(out)),
            Stage::Dop(encoder) => {
                self.words.clear();
                encoder.finish(&mut self.words)?;
                out.extend(self.words.iter().copied().map(f32::from));
                Ok(())
            }
        }
    }
}

/// Mono is copied to every output channel and anything is averaged down to
/// mono. Otherwise channels are kept in order: extra output channels are
/// silent and extra source channels are dropped, which suits stereo music on
/// multichannel devices but is not a surround downmix.
pub(crate) fn map_channels(input: &[f32], from: usize, to: usize, out: &mut Vec<f32>) {
    if from == to {
        out.extend_from_slice(input);
        return;
    }
    let frames = input.chunks_exact(from);
    out.reserve(frames.len() * to);
    for frame in frames {
        if from == 1 {
            out.extend(std::iter::repeat_n(frame[0], to));
        } else if to == 1 {
            out.push(frame.iter().sum::<f32>() / from as f32);
        } else {
            let kept = from.min(to);
            out.extend_from_slice(&frame[..kept]);
            out.extend(std::iter::repeat_n(0.0, to - kept));
        }
    }
}

/// Fixed-ratio FFT resampler over interleaved samples, with its start-up
/// delay trimmed and its tail flushed so the output length matches the input.
struct RateConverter {
    resampler: Fft<f32>,
    channels: usize,
    ratio: f64,
    input: Vec<f32>,
    output: Vec<f32>,
    delay_frames: usize,
    frames_in: u64,
    frames_out: u64,
}

impl RateConverter {
    fn new(from: u32, to: u32, channels: usize, resampling: Resampling) -> Result<Self, Error> {
        let fft_size = resampling.fft_size() as usize;
        // Rubato takes the FFT size from `chunk / sub_chunks`, so a chunk
        // smaller than the FFT could not ask for it.
        let chunk = RESAMPLE_CHUNK.max(fft_size);
        let resampler = Fft::<f32>::new_custom(
            from as usize,
            to as usize,
            chunk,
            chunk / fft_size,
            channels,
            resampling.window().into(),
            FixedSync::Input,
        )
        .map_err(|error| Error::Resample(error.to_string()))?;
        let delay_frames = resampler.output_delay();
        Ok(Self {
            resampler,
            channels,
            ratio: f64::from(to) / f64::from(from),
            input: Vec::new(),
            output: Vec::new(),
            delay_frames,
            frames_in: 0,
            frames_out: 0,
        })
    }

    /// A sample that is not a finite number counts as silence: it would panic
    /// inside the FFT.
    fn push(&mut self, samples: &[f32], out: &mut Vec<f32>) -> Result<(), Error> {
        self.input.extend(
            samples
                .iter()
                .map(|sample| if sample.is_finite() { *sample } else { 0.0 }),
        );
        self.frames_in += (samples.len() / self.channels) as u64;
        let mut consumed = 0;
        loop {
            let needed = self.resampler.input_frames_next();
            let available = (self.input.len() - consumed) / self.channels;
            if available < needed {
                break;
            }
            let start = consumed;
            consumed += self.chunk(start, needed, None, out)?;
        }
        self.input.drain(..consumed);
        Ok(())
    }

    fn expected_frames(&self) -> u64 {
        (self.frames_in as f64 * self.ratio).round() as u64
    }

    fn undelivered_frames(&self) -> u64 {
        self.expected_frames().saturating_sub(self.frames_out)
    }

    fn finish(&mut self, out: &mut Vec<f32>) -> Result<(), Error> {
        let expected = self.expected_frames();
        let remaining = self.input.len() / self.channels;
        if remaining > 0 {
            self.chunk(0, remaining, Some(remaining), out)?;
            self.input.clear();
        }
        // Push silence through until the delayed tail of the real input is
        // out. A call can return nothing while the resampler collects a whole
        // FFT block, which at some ratios spans several chunks, so stop on a
        // bound covering the delay plus one block rather than on empty output.
        let missing = expected.saturating_sub(self.frames_out) as f64 / self.ratio;
        let silence_frames = missing.ceil() as usize + self.resampler.fft_size_in();
        let max_calls = silence_frames.div_ceil(self.resampler.input_frames_next().max(1)) + 1;
        for _ in 0..max_calls {
            if self.frames_out >= expected {
                break;
            }
            self.chunk(0, 0, Some(0), out)?;
        }
        out.truncate(
            out.len() - (self.frames_out.saturating_sub(expected) as usize * self.channels),
        );
        Ok(())
    }

    /// Resamples `frames` frames starting at sample `start` of the input
    /// buffer. Returns the number of input samples consumed.
    fn chunk(
        &mut self,
        start: usize,
        frames: usize,
        partial: Option<usize>,
        out: &mut Vec<f32>,
    ) -> Result<usize, Error> {
        let resample_error = |error: &dyn std::fmt::Display| Error::Resample(error.to_string());
        let channels = self.channels;
        let output_frames = self.resampler.output_frames_next();
        self.output.resize(output_frames * channels, 0.0);
        let input = &self.input[start..start + frames * channels];
        let input = InterleavedSlice::new(input, channels, frames)
            .map_err(|error| resample_error(&error))?;
        let mut output = InterleavedSlice::new_mut(&mut self.output, channels, output_frames)
            .map_err(|error| resample_error(&error))?;
        let indexing = partial.map(|frames| Indexing::new().partial_len(frames));
        let (read, written) = self
            .resampler
            .process_into_buffer(&input, &mut output, indexing.as_ref())
            .map_err(|error| resample_error(&error))?;
        let skipped = self.delay_frames.min(written);
        self.delay_frames -= skipped;
        out.extend_from_slice(&self.output[skipped * channels..written * channels]);
        self.frames_out += (written - skipped) as u64;
        Ok(read.min(frames) * channels)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::ResamplingWindow;

    fn mapped(input: &[f32], from: usize, to: usize) -> Vec<f32> {
        let mut out = Vec::new();
        map_channels(input, from, to, &mut out);
        out
    }

    #[test]
    fn mono_is_copied_to_every_channel() {
        assert_eq!(mapped(&[0.5, -0.5], 1, 2), [0.5, 0.5, -0.5, -0.5]);
    }

    #[test]
    fn stereo_is_averaged_to_mono() {
        assert_eq!(mapped(&[1.0, 0.0, 0.5, 0.5], 2, 1), [0.5, 0.5]);
    }

    #[test]
    fn stereo_on_quad_device_leaves_extra_channels_silent() {
        assert_eq!(mapped(&[0.1, 0.2], 2, 4), [0.1, 0.2, 0.0, 0.0]);
    }

    #[test]
    fn identical_layout_is_untouched() {
        assert_eq!(mapped(&[0.1, 0.2, 0.3, 0.4], 2, 2), [0.1, 0.2, 0.3, 0.4]);
    }

    fn sine(rate: u32, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|frame| (frame as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin())
            .collect()
    }

    #[test]
    fn resampled_length_follows_the_ratio() {
        let source = Spec {
            rate: 44_100,
            channels: 1,
        };
        let output = Spec {
            rate: 48_000,
            channels: 1,
        };
        let mut converter = Converter::new(source, output, Resampling::default()).unwrap();
        let input = sine(44_100, 44_100);
        let mut out = Vec::new();
        // Uneven pushes, as packets of different sizes would arrive
        for chunk in input.chunks(1_000) {
            converter.process(chunk, &mut out).unwrap();
        }
        converter.finish(&mut out).unwrap();
        assert_eq!(out.len(), 48_000);
    }

    /// What has come out plus what is still inside is the length of the input
    /// so far, so the sample where a next track starts can be told in advance.
    #[test]
    fn undelivered_frames_complete_the_output_so_far() {
        let source = Spec {
            rate: 44_100,
            channels: 1,
        };
        let output = Spec {
            rate: 48_000,
            channels: 1,
        };
        let mut converter = Converter::new(source, output, Resampling::default()).unwrap();
        let mut out = Vec::new();
        for chunk in sine(44_100, 22_050).chunks(1_000) {
            converter.process(chunk, &mut out).unwrap();
        }
        let undelivered = converter.undelivered_frames();
        assert!(undelivered > 0, "the resampler holds no input back");
        assert_eq!(out.len() as u64 + undelivered, 24_000);
        converter.finish(&mut out).unwrap();
        assert_eq!(out.len(), 24_000);
        assert_eq!(converter.undelivered_frames(), 0);

        let mut unchanged = Converter::new(source, source, Resampling::default()).unwrap();
        unchanged.process(&[0.5; 100], &mut Vec::new()).unwrap();
        assert_eq!(unchanged.undelivered_frames(), 0);
    }

    #[test]
    fn resampling_preserves_the_signal_level() {
        let source = Spec {
            rate: 96_000,
            channels: 1,
        };
        let output = Spec {
            rate: 48_000,
            channels: 1,
        };
        let mut converter = Converter::new(source, output, Resampling::default()).unwrap();
        let mut out = Vec::new();
        converter.process(&sine(96_000, 96_000), &mut out).unwrap();
        converter.finish(&mut out).unwrap();
        // Skip the edges, where the filter ramps in and out
        let middle = &out[4_800..43_200];
        let peak = middle
            .iter()
            .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
        assert!((0.95..=1.05).contains(&peak), "peak {peak}");
    }

    /// Ratios whose FFT block is larger than one chunk return nothing for
    /// several calls; the tail must still come out whole.
    #[test]
    fn every_ratio_keeps_the_whole_track() {
        let rates = [
            (11_127, 48_000),
            (11_025, 48_000),
            (8_000, 44_100),
            (44_100, 48_000),
            (48_000, 44_100),
            (192_000, 44_100),
            (22_050, 96_000),
            (96_000, 88_200),
            (44_100, 705_600),
            (44_100, 768_000),
            (768_000, 44_100),
        ];
        for (from, to) in rates {
            for frames in [1, 10, 1_000, 4_096, 44_100] {
                let source = Spec {
                    rate: from,
                    channels: 2,
                };
                let output = Spec {
                    rate: to,
                    channels: 2,
                };
                let mut converter = Converter::new(source, output, Resampling::default()).unwrap();
                let mut out = Vec::new();
                for chunk in vec![0.25; frames * 2].chunks(2 * 333) {
                    converter.process(chunk, &mut out).unwrap();
                }
                converter.finish(&mut out).unwrap();
                let expected = (frames as f64 * f64::from(to) / f64::from(from)).round() as usize;
                assert_eq!(
                    out.len() / 2,
                    expected,
                    "{from} -> {to} Hz, {frames} frames"
                );
            }
        }
    }

    #[test]
    fn the_fft_size_reaches_the_resampler() {
        let fft_size_in = |fft_size| {
            let resampling = Resampling::new(fft_size, ResamplingWindow::default());
            RateConverter::new(44_100, 48_000, 2, resampling)
                .unwrap()
                .resampler
                .fft_size_in()
        };
        assert_eq!(
            [256, 1024, 4096, 16_384].map(fft_size_in),
            [294, 1029, 4116, 16_464]
        );
    }

    #[test]
    fn the_window_reaches_the_resampler() {
        let source = Spec {
            rate: 48_000,
            channels: 1,
        };
        let output = Spec {
            rate: 44_100,
            channels: 1,
        };
        let resampled = |window| {
            let resampling = Resampling::new(256, window);
            let mut converter = Converter::new(source, output, resampling).unwrap();
            let mut out = Vec::new();
            converter.process(&sine(48_000, 48_000), &mut out).unwrap();
            converter.finish(&mut out).unwrap();
            out
        };
        assert_ne!(
            resampled(ResamplingWindow::Hann),
            resampled(ResamplingWindow::BlackmanHarris2)
        );
    }

    /// A larger FFT holds more of the input back and takes more of it at once,
    /// and a block can then be larger than a whole short track.
    #[test]
    fn every_fft_size_keeps_the_whole_track_and_its_level() {
        let source = Spec {
            rate: 44_100,
            channels: 1,
        };
        let output = Spec {
            rate: 48_000,
            channels: 1,
        };
        for fft_size in [256, 512, 1024, 2048, 4096, 8192, 16_384] {
            let resampling = Resampling::new(fft_size, ResamplingWindow::default());
            for frames in [100, 44_100] {
                let mut converter = Converter::new(source, output, resampling).unwrap();
                let mut out = Vec::new();
                for chunk in sine(44_100, frames).chunks(1_000) {
                    converter.process(chunk, &mut out).unwrap();
                }
                let delivered = out.len() as u64;
                assert_eq!(
                    delivered + converter.undelivered_frames(),
                    (frames as f64 * 48_000.0 / 44_100.0).round() as u64,
                    "FFT size {fft_size}, {frames} frames"
                );
                converter.finish(&mut out).unwrap();
                let expected = (frames as f64 * 48_000.0 / 44_100.0).round() as usize;
                assert_eq!(out.len(), expected, "FFT size {fft_size}, {frames} frames");
                if frames == 44_100 {
                    let middle = &out[12_000..36_000];
                    let peak = middle
                        .iter()
                        .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
                    assert!((0.95..=1.05).contains(&peak), "FFT size {fft_size}: {peak}");
                }
            }
        }
    }

    #[test]
    fn reset_keeps_the_fft_size() {
        let source = Spec {
            rate: 44_100,
            channels: 1,
        };
        let output = Spec {
            rate: 48_000,
            channels: 1,
        };
        let resampling = Resampling::new(4096, ResamplingWindow::default());
        let mut converter = Converter::new(source, output, resampling).unwrap();
        converter.reset().unwrap();
        let Stage::Pcm {
            resampler: Some(resampler),
            ..
        } = &converter.stage
        else {
            panic!("expected a resampler");
        };
        assert_eq!(resampler.resampler.fft_size_in(), 4116);
    }

    #[test]
    fn samples_that_are_no_numbers_do_not_stop_the_resampler() {
        let spec = |rate| Spec { rate, channels: 2 };
        let mut converter =
            Converter::new(spec(44_100), spec(48_000), Resampling::default()).unwrap();
        let mut input = vec![0.25; 2 * 4_410];
        input[1_000] = f32::NAN;
        input[1_001] = f32::INFINITY;
        let mut out = Vec::new();
        converter.process(&input, &mut out).unwrap();
        converter.finish(&mut out).unwrap();
        assert_eq!(out.len() / 2, 4_800);
        assert!(out.iter().all(|sample| sample.is_finite()));
    }

    #[test]
    fn short_input_is_flushed_on_finish() {
        let source = Spec {
            rate: 48_000,
            channels: 2,
        };
        let output = Spec {
            rate: 44_100,
            channels: 2,
        };
        let mut converter = Converter::new(source, output, Resampling::default()).unwrap();
        let mut out = Vec::new();
        converter.process(&vec![0.25; 2 * 100], &mut out).unwrap();
        converter.finish(&mut out).unwrap();
        assert_eq!(out.len() / 2, 92);
    }
}
