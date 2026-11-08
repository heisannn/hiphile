use std::marker::PhantomData;
use std::mem;

use crate::chain::{self, Filtering, StageDesign};
use crate::clock::Ratio;
use crate::dither::Seeds;
use crate::lanes::{ConstantLanes, Gains, Lanes};
use crate::variable::VariableLanes;
use crate::{
    Dither, Engine, Error, Input, IoSpec, Output, Quality, QualitySpec, RuntimeSpec, Sample, Source,
};

/// Frames that a ratio of 1 with a gain other than 1 keeps back until the end
/// of the input, as libsoxr does
const HELD_AT_ONE: usize = 2;

/// How much one call took and gave, in frames
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Progress {
    pub input_frames: usize,
    pub output_frames: usize,
}

/// Converts frames of `I` at one rate to frames of `O` at another.
///
/// Output frame `k` is the input's signal at input frame `k * io_ratio`: the
/// latency of the filters is removed, so the output is delayed only in that it
/// comes out once enough input has gone in. After [`Self::finish`] it ends at
/// `floor(input frames / io_ratio + 0.5)` frames, computed in 64-bit floating
/// point. How the input and output are split into calls does not change a bit
/// of the output.
///
/// With [`QualitySpec::variable_rate`], [`Self::set_io_ratio`] changes the
/// ratio, and each output frame lies a step of the ratio after the one before.
/// The output then ends at the last frame that lies within the input by at
/// least half of its step.
pub struct Resampler<I: Sample = f32, O: Sample = f32> {
    lanes: Box<dyn Lanes<I, O>>,
    setup: Setup,
    input_frames: u64,
    output_frames: u64,
    ended: bool,
    /// Whether [`Self::set_io_ratio`] may set a new ratio of creation
    ratio_open: bool,
    error: Option<Error>,
    clips: u64,
}

/// What a resampler makes its channels from
struct Setup {
    channels: usize,
    quality: QualitySpec,
    io: IoSpec,
    runtime: RuntimeSpec,
    threads: usize,
    dither: Option<Seeds>,
}

impl Setup {
    fn lanes<I: Sample, O: Sample>(&self, ratio: Ratio) -> Box<dyn Lanes<I, O>> {
        let gains = Gains {
            input: self.io.scale / I::FULL_SCALE,
            output: O::FULL_SCALE,
        };
        let (passband_end, stopband_begin) = self.quality.band();
        let filtering = Filtering {
            precision: self.quality.precision,
            passband_end,
            stopband_begin,
            phase_response: self.quality.phase_response,
        };
        let (channels, dither, threads) = (self.channels, self.dither, self.threads);
        match self.quality.engine() {
            Engine::Variable => Box::new(VariableLanes::new(
                channels,
                ratio.value,
                gains,
                dither,
                threads,
            )),
            _ if ratio.is_one() => {
                let hold = if gains.input * gains.output == 1.0 {
                    Vec::new()
                } else {
                    vec![StageDesign::Hold(HELD_AT_ONE)]
                };
                Box::new(ConstantLanes::<f64>::new(
                    &hold, channels, 1.0, gains, dither, threads,
                ))
            }
            Engine::Single => {
                let designs = chain::design::<f32>(ratio, &filtering, &self.runtime);
                Box::new(ConstantLanes::new(
                    &designs,
                    channels,
                    ratio.value,
                    gains,
                    dither,
                    threads,
                ))
            }
            Engine::Double => {
                let designs = chain::design::<f64>(ratio, &filtering, &self.runtime);
                Box::new(ConstantLanes::new(
                    &designs,
                    channels,
                    ratio.value,
                    gains,
                    dither,
                    threads,
                ))
            }
        }
    }
}

impl<I: Sample, O: Sample> Resampler<I, O> {
    /// At [`Quality::High`]. Only the ratio of the two rates matters.
    pub fn new(input_rate: f64, output_rate: f64, channels: usize) -> Result<Self, Error> {
        Self::builder(input_rate, output_rate, channels).build()
    }

    pub fn builder(input_rate: f64, output_rate: f64, channels: usize) -> Builder<I, O> {
        Builder {
            input_rate,
            output_rate,
            channels,
            quality: None,
            io: IoSpec::default(),
            runtime: RuntimeSpec::default(),
            samples: PhantomData,
        }
    }

    /// Takes input and gives the output it completes. Of `input`, it takes no
    /// more than `ceil(output frames * io_ratio)` frames, which `output` has
    /// room for, so that what it holds stays bounded. The rest is for a later
    /// call.
    pub fn resample<'a, 'b, 'c: 'b>(
        &mut self,
        input: impl Into<Input<'a, I>>,
        output: impl Into<Output<'b, 'c, O>>,
    ) -> Result<Progress, Error> {
        self.ratio_open = false;
        if let Some(error) = self.error {
            return Err(error);
        }
        let input = input.into();
        let mut output = output.into();
        let input_frames = input.frames(self.setup.channels)?;
        let room = output.frames(self.setup.channels)?;
        if self.ended && input_frames > 0 {
            return Err(Error::InputAfterEnd);
        }
        let taken = input_frames.min((room as f64 * self.io_ratio()).ceil() as usize);
        if taken > 0 {
            self.lanes.push(&input, taken);
            self.input_frames += taken as u64;
        }
        Ok(Progress {
            input_frames: taken,
            output_frames: self.deliver(&mut output, 0, room),
        })
    }

    /// Gives output until `output` is full or the input has ended, pulling
    /// input from `source` as it is needed. Every pull of a call asks for
    /// `ceil(room × io_ratio)` frames, or the source's most if that is less.
    /// When the source fails after some output, the call returns that output
    /// and the next one the error.
    pub fn output<'b, 'c: 'b>(
        &mut self,
        source: &mut (impl Source<I> + ?Sized),
        output: impl Into<Output<'b, 'c, O>>,
    ) -> Result<usize, Error> {
        self.ratio_open = false;
        if let Some(error) = self.error {
            return Err(error);
        }
        let mut output = output.into();
        let room = output.frames(self.setup.channels)?;
        let needed = (room as f64 * self.io_ratio()).ceil() as usize;
        let request = source
            .most_frames()
            .map_or(needed, |most| needed.min(most.get()));
        let mut written = self.deliver(&mut output, 0, room);
        while written < room && !self.ended {
            let Some(frames) = self.pull(source, request) else {
                self.error = Some(Error::InputFailure);
                return if written == 0 {
                    Err(Error::InputFailure)
                } else {
                    Ok(written)
                };
            };
            self.ended = frames == 0;
            written += self.deliver(&mut output, written, room - written);
        }
        Ok(written)
    }

    /// Takes what `source` gives for a `request`, and returns how many frames
    /// that was, or nothing if it failed
    fn pull(&mut self, source: &mut (impl Source<I> + ?Sized), request: usize) -> Option<usize> {
        let input = source.pull(request).ok()?;
        let frames = input.frames(self.setup.channels).ok()?;
        if frames > request {
            return None;
        }
        if frames > 0 {
            self.lanes.push(&input, frames);
            self.input_frames += frames as u64;
        }
        Some(frames)
    }

    /// Ends the input, as many times as wanted, and gives what is left of the
    /// output, as far as `output` has room. Call again until it gives 0.
    pub fn finish<'b, 'c: 'b>(
        &mut self,
        output: impl Into<Output<'b, 'c, O>>,
    ) -> Result<usize, Error> {
        self.ratio_open = false;
        if let Some(error) = self.error {
            return Err(error);
        }
        let mut output = output.into();
        let room = output.frames(self.setup.channels)?;
        self.ended = true;
        Ok(self.deliver(&mut output, 0, room))
    }

    /// Writes up to `room` frames of output from frame `start` of `output` on
    fn deliver(&mut self, output: &mut Output<'_, '_, O>, start: usize, room: usize) -> usize {
        let left = self.lanes.left(self.input_frames, self.output_frames);
        let wanted = usize::try_from(left).unwrap_or(usize::MAX).min(room);
        self.lanes.fill(wanted, self.ended);
        let frames = self.lanes.queued().min(wanted);
        if frames > 0 {
            self.clips += self.lanes.write(output, start, frames);
            self.output_frames += frames as u64;
        }
        frames
    }

    /// In output frames: before the end of the input, how far the input so
    /// far reaches past the time of the next output frame, and after it the
    /// output still to come
    pub fn delay(&self) -> f64 {
        if self.ended {
            self.lanes.left(self.input_frames, self.output_frames) as f64
        } else {
            self.lanes.reach(self.input_frames, self.output_frames)
        }
    }

    /// Back to the state after creation, with the same settings: the input,
    /// output, error and clips so far are gone, a variable ratio is the ratio
    /// of creation again, and dither starts again with the noise that every
    /// resampler not given a seed starts with after a clear.
    pub fn clear(&mut self) {
        self.lanes.clear();
        self.input_frames = 0;
        self.output_frames = 0;
        self.ended = false;
        self.ratio_open = !self.setup.quality.keep_ratio_on_clear;
        self.error = None;
        self.clips = 0;
    }

    /// Input frames per output frame, over 0 for a variable rate. From the
    /// next output frame on, the ratio moves to it linearly over `slew_len`
    /// frames, or at once if that is 0. Each step of a slew is the ratio
    /// halfway through it, so the slew moves the time by `slew_len` times the
    /// mean of the old ratio and the new. Over 64 the filter widens no more,
    /// so the output aliases.
    ///
    /// A resampler that keeps its ratio accepts only the same one again,
    /// within 5e-16 of it relative to it. Without
    /// [`QualitySpec::keep_ratio_on_clear`], it accepts any ratio once after
    /// [`Self::clear`] and before it next resamples, and resamples at it from
    /// then on. An error that the quality spec has at that ratio stops the
    /// resampler.
    pub fn set_io_ratio(&mut self, io_ratio: f64, slew_len: usize) -> Result<(), Error> {
        if !(io_ratio.is_finite() && io_ratio > 0.0) {
            return Err(Error::IoRatioOutOfRange);
        }
        let open = mem::take(&mut self.ratio_open);
        match self.lanes.set_io_ratio(io_ratio, slew_len) {
            Err(Error::VaryingRatio) if open => self.configure(io_ratio),
            result => result,
        }
    }

    fn configure(&mut self, io_ratio: f64) -> Result<(), Error> {
        if let Err(error) = self.setup.quality.validate(io_ratio < 1.0) {
            self.error = Some(error);
            return Err(error);
        }
        let mut lanes = self.setup.lanes(Ratio::of_rates(io_ratio, 1.0));
        // For the dither after a clear
        lanes.clear();
        self.lanes = lanes;
        Ok(())
    }

    /// Input frames per output frame, from the next output frame to the one
    /// after
    pub fn io_ratio(&self) -> f64 {
        self.lanes.io_ratio()
    }

    pub fn channels(&self) -> usize {
        self.setup.channels
    }

    pub fn engine(&self) -> Engine {
        self.setup.quality.engine()
    }

    pub fn quality(&self) -> &QualitySpec {
        &self.setup.quality
    }

    pub fn io(&self) -> &IoSpec {
        &self.setup.io
    }

    pub fn runtime(&self) -> &RuntimeSpec {
        &self.setup.runtime
    }

    /// Of [`RuntimeSpec::num_threads`], with 0 resolved to the number of cores
    pub fn threads(&self) -> usize {
        self.setup.threads
    }

    /// The error that stops the resampler until [`Self::clear`]
    pub fn error(&self) -> Option<Error> {
        self.error
    }

    /// Integer samples held to their type's range since creation or
    /// [`Self::clear`], over all channels
    pub fn clips(&self) -> u64 {
        self.clips
    }

    pub fn reset_clips(&mut self) {
        self.clips = 0;
    }
}

/// Settings of a [`Resampler`] before it is made. Each has libsoxr's default.
#[must_use]
pub struct Builder<I: Sample, O: Sample> {
    input_rate: f64,
    output_rate: f64,
    channels: usize,
    quality: Option<QualitySpec>,
    io: IoSpec,
    runtime: RuntimeSpec,
    samples: PhantomData<fn(I) -> O>,
}

impl<I: Sample, O: Sample> Builder<I, O> {
    /// [`Quality::High`] by default, and [`Quality::Low`] for [`Self::oneshot`]
    pub fn quality(self, quality: impl Into<QualitySpec>) -> Self {
        Self {
            quality: Some(quality.into()),
            ..self
        }
    }

    pub fn io(self, io: IoSpec) -> Self {
        Self { io, ..self }
    }

    pub fn runtime(self, runtime: RuntimeSpec) -> Self {
        Self { runtime, ..self }
    }

    /// Checks the rates, the channels, the quality and the rest, in that
    /// order, and returns the first error.
    pub fn build(self) -> Result<Resampler<I, O>, Error> {
        let quality = self.quality.unwrap_or_default();
        self.make(quality)
    }

    /// Converts the whole of `input` at once, into interleaved frames.
    pub fn oneshot<'a>(self, input: impl Into<Input<'a, I>>) -> Result<Vec<O>, Error> {
        let quality = self.quality.unwrap_or_else(|| Quality::Low.into());
        let mut resampler = self.make(quality)?;
        let input = input.into();
        let channels = resampler.setup.channels;
        let frames = input.frames(channels)?;
        resampler.lanes.push(&input, frames);
        resampler.input_frames = frames as u64;
        let total = resampler.lanes.left(resampler.input_frames, 0) as usize;
        let mut output = vec![O::default(); total * channels];
        let written = resampler.finish(&mut output[..])?;
        debug_assert_eq!(written, total);
        Ok(output)
    }

    fn make(self, quality: QualitySpec) -> Result<Resampler<I, O>, Error> {
        self.validate(&quality)?;
        let dither = match self.io.dither {
            Dither::Tpdf { seed } if O::TAKES_DITHER => Some(Seeds::new(seed)),
            _ => None,
        };
        let setup = Setup {
            channels: self.channels,
            quality,
            io: self.io,
            runtime: self.runtime,
            threads: self.runtime.threads(),
            dither,
        };
        let lanes = setup.lanes(Ratio::of_rates(self.input_rate, self.output_rate));
        Ok(Resampler {
            lanes,
            setup,
            input_frames: 0,
            output_frames: 0,
            ended: false,
            ratio_open: false,
            error: None,
            clips: 0,
        })
    }

    fn validate(&self, quality: &QualitySpec) -> Result<(), Error> {
        let is_valid_rate = |rate: f64| rate.is_finite() && rate > 0.0;
        if !is_valid_rate(self.input_rate) || !is_valid_rate(self.output_rate) {
            return Err(Error::IoRatioOutOfRange);
        }
        let io_ratio = self.input_rate / self.output_rate;
        if !is_valid_rate(io_ratio) {
            return Err(Error::IoRatioOutOfRange);
        }
        if self.channels == 0 {
            return Err(Error::InvalidChannels);
        }
        quality.validate(io_ratio < 1.0)?;
        if !self.io.scale.is_finite() {
            return Err(Error::Scale);
        }
        Ok(())
    }
}
