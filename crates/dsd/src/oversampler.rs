use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
    Async, Fft, FixedAsync, FixedSync, Indexing, Resampler, SincInterpolationParameters,
    SincInterpolationType,
};

use crate::Error;
use crate::settings::{Filter, ResamplingWindow};

const CHANNELS: usize = 2;
const CHUNK: usize = 1024;
/// The rate of the second stage's input is a whole divisor of every DSD rate.
const SECOND_STAGE_INPUT_FOR_44_1: u32 = 352_800;
const SECOND_STAGE_INPUT_FOR_48: u32 = 384_000;
const SECOND_STAGE_FFT: u32 = 256;
/// Above this, a table with one row per output phase would take gigabytes, as
/// for a rate with no small ratio to the target.
const LARGEST_SINC_TABLE: u32 = 4096;
/// `output_delay` of `Async` says this many output frames too many: trimming
/// all it says makes the output lead the input. It comes to exactly 2 at a
/// whole ratio and about 1 at any other, where a part of a frame always stays,
/// too short to matter at these rates. A test holds it to this, which would
/// catch a Rubato that changes it.
const SINC_DELAY_OVERSTATED_AT_WHOLE_RATIO: usize = 2;
const SINC_DELAY_OVERSTATED_OTHERWISE: usize = 1;

/// Interleaved stereo at the source rate in, at the DSD bit rate out, with the
/// same number of frames per second of input whatever the filter's delay.
pub(crate) struct Oversampler {
    stages: Vec<Stage>,
}

impl Oversampler {
    pub(crate) fn new(from: u32, bit_rate: u32, filter: Filter) -> Result<Self, Error> {
        let stages = match filter {
            Filter::Fft { size, window } => vec![Stage::fft(from, bit_rate, size, window)?],
            Filter::Sinc {
                length,
                window,
                cutoff,
            } => {
                let middle = if bit_rate.is_multiple_of(SECOND_STAGE_INPUT_FOR_44_1) {
                    SECOND_STAGE_INPUT_FOR_44_1
                } else {
                    SECOND_STAGE_INPUT_FOR_48
                };
                if from < middle {
                    vec![
                        Stage::sinc(from, middle, length.taps(), window, cutoff)?,
                        Stage::fft(middle, bit_rate, SECOND_STAGE_FFT, window)?,
                    ]
                } else {
                    vec![Stage::fft(from, bit_rate, SECOND_STAGE_FFT, window)?]
                }
            }
        };
        Ok(Self { stages })
    }

    pub(crate) fn push(&mut self, input: &[f64], out: &mut Vec<f64>) -> Result<(), Error> {
        let mut carried = Vec::new();
        let last = self.stages.len() - 1;
        for (index, stage) in self.stages.iter_mut().enumerate() {
            if index == last {
                let input = if index == 0 { input } else { &carried };
                stage.push(input, out)?;
            } else {
                let mut produced = Vec::new();
                stage.push(if index == 0 { input } else { &carried }, &mut produced)?;
                carried = produced;
            }
        }
        Ok(())
    }

    pub(crate) fn finish(&mut self, out: &mut Vec<f64>) -> Result<(), Error> {
        let mut carried = Vec::new();
        let last = self.stages.len() - 1;
        for (index, stage) in self.stages.iter_mut().enumerate() {
            if index == last {
                stage.push(&carried, out)?;
                stage.finish(out)?;
            } else {
                let mut produced = Vec::new();
                stage.push(&carried, &mut produced)?;
                stage.finish(&mut produced)?;
                carried = produced;
            }
        }
        Ok(())
    }

    /// Frames at the bit rate that the input so far comes to, still inside.
    pub(crate) fn undelivered_frames(&self) -> u64 {
        let mut undelivered = 0.0;
        for stage in &self.stages {
            undelivered = undelivered * stage.ratio + stage.undelivered_frames() as f64;
        }
        undelivered.round() as u64
    }
}

struct Stage {
    resampler: Box<dyn Resampler<f64>>,
    ratio: f64,
    input: Vec<f64>,
    output: Vec<f64>,
    skip_frames: usize,
    frames_in: u64,
    frames_out: u64,
}

impl Stage {
    fn sinc(
        from: u32,
        to: u32,
        taps: usize,
        window: ResamplingWindow,
        cutoff: Option<f32>,
    ) -> Result<Self, Error> {
        let table = to / gcd(from, to);
        let interpolation = if table <= LARGEST_SINC_TABLE {
            SincInterpolationType::Nearest
        } else {
            SincInterpolationType::Cubic
        };
        let factor = table.min(LARGEST_SINC_TABLE) as usize;
        let mut parameters = SincInterpolationParameters::new(taps, window.into())
            .interpolation(interpolation)
            .oversampling_factor(factor);
        if let Some(cutoff) = cutoff {
            parameters = parameters.f_cutoff(cutoff);
        }
        let ratio = f64::from(to) / f64::from(from);
        let resampler =
            Async::<f64>::new_sinc(ratio, 1.0, &parameters, CHUNK, CHANNELS, FixedAsync::Input)
                .map_err(resample_error)?;
        let overstated = if to.is_multiple_of(from) {
            SINC_DELAY_OVERSTATED_AT_WHOLE_RATIO
        } else {
            SINC_DELAY_OVERSTATED_OTHERWISE
        };
        let skip_frames = resampler.output_delay() - overstated;
        Ok(Self::around(Box::new(resampler), ratio, skip_frames))
    }

    fn fft(from: u32, to: u32, size: u32, window: ResamplingWindow) -> Result<Self, Error> {
        let size = size.max(1) as usize;
        // Rubato takes the FFT size from `chunk / sub_chunks`, so a chunk
        // smaller than the FFT could not ask for it.
        let chunk = CHUNK.max(size);
        let resampler = Fft::<f64>::new_custom(
            from as usize,
            to as usize,
            chunk,
            chunk / size,
            CHANNELS,
            window.into(),
            FixedSync::Input,
        )
        .map_err(resample_error)?;
        let skip_frames = resampler.output_delay();
        let ratio = f64::from(to) / f64::from(from);
        Ok(Self::around(Box::new(resampler), ratio, skip_frames))
    }

    fn around(resampler: Box<dyn Resampler<f64>>, ratio: f64, skip_frames: usize) -> Self {
        Self {
            resampler,
            ratio,
            input: Vec::new(),
            output: Vec::new(),
            skip_frames,
            frames_in: 0,
            frames_out: 0,
        }
    }

    fn push(&mut self, samples: &[f64], out: &mut Vec<f64>) -> Result<(), Error> {
        self.input.extend_from_slice(samples);
        self.frames_in += (samples.len() / CHANNELS) as u64;
        let mut consumed = 0;
        loop {
            let needed = self.resampler.input_frames_next();
            if (self.input.len() - consumed) / CHANNELS < needed {
                break;
            }
            consumed += self.chunk(consumed, needed, None, out)?;
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

    fn finish(&mut self, out: &mut Vec<f64>) -> Result<(), Error> {
        let expected = self.expected_frames();
        let remaining = self.input.len() / CHANNELS;
        if remaining > 0 {
            self.chunk(0, remaining, Some(remaining), out)?;
            self.input.clear();
        }
        // A call can return nothing while the resampler collects a whole FFT
        // block, so stop on a bound covering the delay rather than on empty
        // output.
        let missing = expected.saturating_sub(self.frames_out) as f64 / self.ratio;
        let silence_frames = missing.ceil() as usize
            + self.skip_frames
            + self.resampler.output_delay()
            + self.resampler.input_frames_next();
        let max_calls = silence_frames.div_ceil(self.resampler.input_frames_next().max(1)) + 1;
        for _ in 0..max_calls {
            if self.frames_out >= expected {
                break;
            }
            self.chunk(0, 0, Some(0), out)?;
        }
        out.truncate(out.len() - self.frames_out.saturating_sub(expected) as usize * CHANNELS);
        self.frames_out = self.frames_out.min(expected);
        Ok(())
    }

    /// Resamples `frames` frames starting at sample `start` of the input
    /// buffer. Returns the number of input samples consumed.
    fn chunk(
        &mut self,
        start: usize,
        frames: usize,
        partial: Option<usize>,
        out: &mut Vec<f64>,
    ) -> Result<usize, Error> {
        let output_frames = self.resampler.output_frames_next();
        self.output.resize(output_frames * CHANNELS, 0.0);
        let input = &self.input[start..start + frames * CHANNELS];
        let input = InterleavedSlice::new(input, CHANNELS, frames).map_err(resample_error)?;
        let mut output = InterleavedSlice::new_mut(&mut self.output, CHANNELS, output_frames)
            .map_err(resample_error)?;
        let indexing = partial.map(|frames| Indexing::new().partial_len(frames));
        let (read, written) = self
            .resampler
            .process_into_buffer(&input, &mut output, indexing.as_ref())
            .map_err(resample_error)?;
        let skipped = self.skip_frames.min(written);
        self.skip_frames -= skipped;
        out.extend_from_slice(&self.output[skipped * CHANNELS..written * CHANNELS]);
        self.frames_out += (written - skipped) as u64;
        Ok(read.min(frames) * CHANNELS)
    }
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

fn resample_error(error: impl std::fmt::Display) -> Error {
    Error::Resample(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::SincLength;

    const SINC_256: Filter = Filter::Sinc {
        length: SincLength::L256,
        window: ResamplingWindow::BlackmanHarris2,
        cutoff: None,
    };
    const FFT_256: Filter = Filter::Fft {
        size: 256,
        window: ResamplingWindow::BlackmanHarris2,
    };

    fn run(from: u32, to: u32, filter: Filter, input: &[f64]) -> Vec<f64> {
        let mut oversampler = Oversampler::new(from, to, filter).unwrap();
        let mut out = Vec::new();
        for block in input.chunks(2 * 777) {
            oversampler.push(block, &mut out).unwrap();
        }
        oversampler.finish(&mut out).unwrap();
        out
    }

    fn sine(rate: u32, frames: usize) -> Vec<f64> {
        (0..frames)
            .flat_map(|index| {
                let value =
                    (std::f64::consts::TAU * 1_000.0 * index as f64 / f64::from(rate)).sin();
                [value, -value]
            })
            .collect()
    }

    fn check_sine(out: &[f64], input_rate: u32, output_rate: u32, tolerance: f64) {
        let ratio = f64::from(output_rate) / f64::from(input_rate);
        // Past both ends, where the filter has only half its input
        let margin = (8_000.0 * ratio) as usize;
        for (index, frame) in out.as_chunks::<2>().0.iter().enumerate().skip(margin) {
            if index + margin >= out.len() / 2 {
                break;
            }
            let wanted =
                (std::f64::consts::TAU * 1_000.0 * index as f64 / f64::from(output_rate)).sin();
            assert!(
                (frame[0] - wanted).abs() < tolerance,
                "{index}: {}",
                frame[0]
            );
            assert!(
                (frame[1] + wanted).abs() < tolerance,
                "{index}: {}",
                frame[1]
            );
        }
    }

    #[test]
    fn output_has_the_length_and_the_timing_of_the_input() {
        for (from, to, tolerance) in [
            (44_100, 2_822_400, 1e-3),
            (96_000, 3_072_000, 1e-3),
            (192_000, 3_072_000, 1e-3),
            (44_100, 3_072_000, 2e-2),
            (48_000, 11_289_600, 2e-2),
            (96_000, 2_822_400, 2e-2),
            (192_000, 11_289_600, 2e-2),
        ] {
            for filter in [SINC_256, FFT_256] {
                let frames = 3 * from as usize / 2;
                let out = run(from, to, filter, &sine(from, frames));
                let expected =
                    (frames as f64 * f64::from(to) / f64::from(from)).round() as usize * 2;
                assert_eq!(out.len(), expected, "{from} {to} {filter:?}");
                check_sine(&out, from, to, tolerance);
            }
        }
    }

    #[test]
    fn nothing_is_left_inside_after_finishing() {
        let mut oversampler = Oversampler::new(44_100, 2_822_400, SINC_256).unwrap();
        let mut out = Vec::new();
        oversampler.push(&sine(44_100, 10_000), &mut out).unwrap();
        assert!(oversampler.undelivered_frames() > 0);
        oversampler.finish(&mut out).unwrap();
        assert_eq!(oversampler.undelivered_frames(), 0);
    }

    #[test]
    fn a_source_above_the_middle_rate_goes_straight_up() {
        let frames = 192_000;
        let out = run(384_000, 3_072_000, SINC_256, &sine(384_000, frames));
        assert_eq!(out.len(), frames * 8 * 2);
        check_sine(&out, 384_000, 3_072_000, 1e-3);
    }
}
