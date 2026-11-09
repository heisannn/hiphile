use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Indexing, Resampler as _};

use self::r8brain_converter::R8brainConverter;
use self::soxr_converter::SoxrConverter;
use crate::Error;
use crate::output::{OutputSettings, Resampling, ResamplingWindow, Spec};

mod r8brain_converter;
mod soxr_converter;

pub(crate) struct Converter {
    source: Spec,
    output: Spec,
    stage: Stage,
    dsp: Dsp,
    mapped: Vec<f32>,
    words: Vec<u16>,
}

enum Stage {
    Pcm {
        resampling: Resampling,
        resampler: Option<RateConverter>,
    },
    /// Its samples are DSD words as whole numbers, which `f32` holds exactly.
    Dop(Box<dsd::Encoder>),
}

/// Applies the settings in `settings`, [`dsp::Settings::to_bits`], to the
/// track at its own rate, after the channel map and before the resampler or
/// the encoder. The processor has no delay, so the timing stays as it is.
struct Dsp {
    settings: Arc<AtomicU64>,
    processor: dsp::Processor,
}

impl Dsp {
    fn new(settings: Arc<AtomicU64>, source: Spec, output: Spec) -> Self {
        let processor = dsp::Processor::new(
            source.rate,
            usize::from(output.channels),
            dsp::Settings::from_bits(settings.load(Ordering::Relaxed)),
        );
        Self {
            settings,
            processor,
        }
    }

    /// Goes on from where this one is, so that a joint has no step.
    fn following(&self, source: Spec) -> Self {
        Self {
            settings: Arc::clone(&self.settings),
            processor: self.processor.clone().at_rate(source.rate),
        }
    }

    fn process(&mut self, samples: &mut [f32]) {
        self.processor.follow(dsp::Settings::from_bits(
            self.settings.load(Ordering::Relaxed),
        ));
        self.processor.process(samples);
    }
}

impl Converter {
    #[cfg(test)]
    pub(crate) fn new(source: Spec, output: Spec, resampling: Resampling) -> Result<Self, Error> {
        let settings = OutputSettings::default().with_resampling(resampling);
        Self::for_settings(source, output, &settings, &Arc::default())
    }

    /// As `settings` say, for `source` on a stream of `output`, with the DSP
    /// settings that `dsp` holds then and later.
    pub(crate) fn for_settings(
        source: Spec,
        output: Spec,
        settings: &OutputSettings,
        dsp: &Arc<AtomicU64>,
    ) -> Result<Self, Error> {
        let stage = Self::stage(source, output, settings)?;
        let dsp = Dsp::new(Arc::clone(dsp), source, output);
        Ok(Self::around(source, output, stage, dsp))
    }

    /// For the track that comes on after this one, on the same stream.
    pub(crate) fn following(&self, source: Spec, settings: &OutputSettings) -> Result<Self, Error> {
        let stage = match &self.stage {
            Stage::Pcm { .. } => Self::stage(source, self.output, settings)?,
            Stage::Dop(encoder) => Stage::Dop(Box::new(encoder.for_next_track(source.rate)?)),
        };
        Ok(Self::around(
            source,
            self.output,
            stage,
            self.dsp.following(source),
        ))
    }

    fn stage(source: Spec, output: Spec, settings: &OutputSettings) -> Result<Stage, Error> {
        Ok(match settings.dop() {
            None => {
                let resampling = settings.resampling();
                Stage::Pcm {
                    resampling,
                    resampler: Self::resampler(source, output, resampling)?,
                }
            }
            Some(dop) => Stage::Dop(Box::new(dsd::Encoder::new(source.rate, output.rate, dop)?)),
        })
    }

    fn around(source: Spec, output: Spec, stage: Stage, dsp: Dsp) -> Self {
        Self {
            source,
            output,
            stage,
            dsp,
            mapped: Vec::new(),
            words: Vec::new(),
        }
    }

    fn resampler(
        source: Spec,
        output: Spec,
        resampling: Resampling,
    ) -> Result<Option<RateConverter>, Error> {
        (source.rate != output.rate)
            .then(|| {
                RateConverter::new(
                    source.rate,
                    output.rate,
                    usize::from(output.channels),
                    resampling,
                )
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
                let start = out.len();
                map_channels(input, from, to, out);
                self.dsp.process(&mut out[start..]);
                Ok(())
            }
            Stage::Pcm {
                resampler: Some(resampler),
                ..
            } => {
                self.mapped.clear();
                map_channels(input, from, to, &mut self.mapped);
                self.dsp.process(&mut self.mapped);
                resampler.push(&self.mapped, out)
            }
            Stage::Dop(encoder) => {
                self.mapped.clear();
                map_channels(input, from, to, &mut self.mapped);
                self.dsp.process(&mut self.mapped);
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

enum RateConverter {
    Fft(Box<FftConverter>),
    R8brain(R8brainConverter),
    Soxr(Box<SoxrConverter>),
}

impl RateConverter {
    fn new(from: u32, to: u32, channels: usize, resampling: Resampling) -> Result<Self, Error> {
        match resampling {
            Resampling::Fft { fft_size, window } => {
                FftConverter::new(from, to, channels, fft_size, window)
                    .map(|converter| Self::Fft(Box::new(converter)))
            }
            Resampling::R8brain(settings) => {
                R8brainConverter::new(from, to, channels, settings).map(Self::R8brain)
            }
            Resampling::Soxr(settings) => SoxrConverter::new(from, to, channels, settings)
                .map(|converter| Self::Soxr(Box::new(converter))),
        }
    }

    fn push(&mut self, samples: &[f32], out: &mut Vec<f32>) -> Result<(), Error> {
        match self {
            Self::Fft(converter) => converter.push(samples, out),
            Self::R8brain(converter) => {
                converter.push(samples, out);
                Ok(())
            }
            Self::Soxr(converter) => converter.push(samples, out),
        }
    }

    fn undelivered_frames(&self) -> u64 {
        match self {
            Self::Fft(converter) => converter.undelivered_frames(),
            Self::R8brain(converter) => converter.undelivered_frames(),
            Self::Soxr(converter) => converter.undelivered_frames(),
        }
    }

    fn finish(&mut self, out: &mut Vec<f32>) -> Result<(), Error> {
        match self {
            Self::Fft(converter) => converter.finish(out),
            Self::R8brain(converter) => {
                converter.finish(out);
                Ok(())
            }
            Self::Soxr(converter) => converter.finish(out),
        }
    }
}

/// Fixed-ratio FFT resampler over interleaved samples, with its start-up
/// delay trimmed and its tail flushed so the output length matches the input.
struct FftConverter {
    resampler: Fft<f32>,
    channels: usize,
    ratio: f64,
    input: Vec<f32>,
    output: Vec<f32>,
    delay_frames: usize,
    frames_in: u64,
    frames_out: u64,
}

impl FftConverter {
    fn new(
        from: u32,
        to: u32,
        channels: usize,
        fft_size: u32,
        window: ResamplingWindow,
    ) -> Result<Self, Error> {
        if from == 0 || to == 0 {
            let error = rubato::ResamplerConstructionError::InvalidSampleRate {
                input: from as usize,
                output: to as usize,
            };
            return Err(Error::Resample(error.to_string()));
        }
        let resampler = Fft::<f32>::new_custom(
            from as usize,
            to as usize,
            input_block(from, to, fft_size),
            1,
            channels,
            window.into(),
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
        // out, within a bound covering the delay plus one block
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

/// Input frames of one FFT block whose side at the lower rate has at least `fft_size` frames.
/// Rubato sets the cutoff by the shorter side, so sizing the input side alone would leave a large
/// fall in rate a short output block and a cutoff well under the Nyquist frequency.
fn input_block(from: u32, to: u32, fft_size: u32) -> usize {
    let gcd = gcd(from, to);
    let blocks = fft_size.max(1).div_ceil(from.min(to) / gcd);
    blocks as usize * (from / gcd) as usize
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

#[cfg(test)]
mod tests {
    use crate::output::{R8brainPhase, R8brainSettings, SoxrSettings};

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

    fn each_resampling() -> [Resampling; 3] {
        [
            Resampling::default(),
            Resampling::R8brain(R8brainSettings::default()),
            Resampling::Soxr(SoxrSettings::default()),
        ]
    }

    fn fft_size_in(converter: &RateConverter) -> usize {
        match converter {
            RateConverter::Fft(converter) => converter.resampler.fft_size_in(),
            RateConverter::R8brain(_) | RateConverter::Soxr(_) => {
                panic!("expected the FFT resampler")
            }
        }
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
        for resampling in each_resampling() {
            let mut converter = Converter::new(source, output, resampling).unwrap();
            let input = sine(44_100, 44_100);
            let mut out = Vec::new();
            // Uneven pushes, as packets of different sizes would arrive
            for chunk in input.chunks(1_000) {
                converter.process(chunk, &mut out).unwrap();
            }
            converter.finish(&mut out).unwrap();
            assert_eq!(out.len(), 48_000);
        }
    }

    /// The first frame out is the first frame in, whatever r8brain holds back,
    /// and each channel stays its own.
    #[test]
    fn r8brain_keeps_the_time_and_the_channels_of_the_input() {
        let spec = |rate| Spec { rate, channels: 2 };
        let mut converter = Converter::new(
            spec(44_100),
            spec(48_000),
            Resampling::R8brain(R8brainSettings::default()),
        )
        .unwrap();
        // In `f64`, as the phase of `sine` drifts from rounding, too little to
        // hear but more than this test allows
        let wave =
            |frame: usize, rate: f64| (frame as f64 * 440.0 * std::f64::consts::TAU / rate).sin();
        let input: Vec<f32> = (0..44_100)
            .flat_map(|frame| [wave(frame, 44_100.0) as f32, 0.0])
            .collect();
        let mut out = Vec::new();
        for chunk in input.chunks(2 * 333) {
            converter.process(chunk, &mut out).unwrap();
        }
        converter.finish(&mut out).unwrap();

        let tail_start = 47_000;
        let frames = out.as_chunks::<2>().0.iter().enumerate();
        for (frame, samples) in frames.take(tail_start).skip(2_000) {
            let error = (f64::from(samples[0]) - wave(frame, 48_000.0)).abs();
            assert!(error < 1e-5, "frame {frame} off by {error}");
            assert_eq!(samples[1], 0.0, "frame {frame}");
        }
    }

    #[test]
    fn every_r8brain_option_keeps_the_length_and_the_level() {
        let spec = |rate| Spec { rate, channels: 2 };
        let changes: [fn(&mut R8brainSettings); 9] = [
            |settings| settings.options.phase = R8brainPhase::Minimum,
            |settings| settings.options.transition_band = 20.0,
            |settings| settings.options.attenuation = 100.0,
            |settings| settings.options.extended_fft = true,
            |settings| settings.options.whole_stepping = false,
            |settings| {
                settings.options.whole_stepping = false;
                settings.options.filter_fractions = Some(64);
            },
            |settings| settings.options.max_input_len = 1,
            |settings| settings.options.max_input_len = 1 << 16,
            |settings| {
                settings.cache_limits.filters = 2;
                settings.cache_limits.filter_banks = 2;
            },
        ];
        for change in changes {
            let mut settings = R8brainSettings::default();
            change(&mut settings);
            let resampling = Resampling::R8brain(settings);
            let mut converter = Converter::new(spec(44_100), spec(48_000), resampling).unwrap();
            let input: Vec<f32> = sine(44_100, 44_100)
                .into_iter()
                .flat_map(|sample| [sample, -sample])
                .collect();
            let mut out = Vec::new();
            for chunk in input.chunks(2 * 333) {
                converter.process(chunk, &mut out).unwrap();
            }
            converter.finish(&mut out).unwrap();

            assert_eq!(out.len(), 2 * 48_000, "{settings:?}");
            let middle = &out[2 * 20_000..2 * 28_000];
            let peak = middle
                .iter()
                .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
            assert!((peak - 1.0).abs() < 0.01, "{settings:?}: {peak}");
        }
        r8brain::CacheLimits::default().apply().unwrap();
    }

    #[test]
    fn a_large_r8brain_input_block_adds_only_the_tail_on_finish() {
        let spec = |rate| Spec { rate, channels: 2 };
        let mut settings = R8brainSettings::default();
        settings.options.max_input_len = 1 << 18;
        let resampling = Resampling::R8brain(settings);
        let mut converter = Converter::new(spec(44_100), spec(176_400), resampling).unwrap();
        let mut out = Vec::new();
        converter.process(&vec![0.25; 2 * 1_000], &mut out).unwrap();
        converter.finish(&mut out).unwrap();
        assert_eq!(out.len(), 2 * 4_000);
        assert!(
            out.capacity() < 2 * 40_000,
            "room for {} samples",
            out.capacity()
        );
    }

    #[test]
    fn a_setting_outside_the_range_of_r8brain_is_an_error() {
        let spec = |rate| Spec { rate, channels: 2 };
        let mut settings = R8brainSettings::default();
        settings.options.attenuation = 10.0;
        assert!(Converter::new(spec(44_100), spec(48_000), Resampling::R8brain(settings)).is_err());
    }

    #[test]
    fn r8brain_starts_over_on_a_reset() {
        let spec = |rate| Spec { rate, channels: 1 };
        let mut converter = Converter::new(
            spec(44_100),
            spec(48_000),
            Resampling::R8brain(R8brainSettings::default()),
        )
        .unwrap();
        let input = sine(44_100, 8_000);
        let mut first = Vec::new();
        converter.process(&input, &mut first).unwrap();
        assert!(converter.undelivered_frames() > 0);

        converter.reset().unwrap();
        let mut again = Vec::new();
        converter.process(&input, &mut again).unwrap();
        assert_eq!(first, again);
    }

    /// As with r8brain, at HQ, which computes in `f32`, and at VHQ, in `f64`
    #[test]
    fn soxr_keeps_the_time_and_the_channels_of_the_input() {
        let spec = |rate| Spec { rate, channels: 2 };
        let wave =
            |frame: usize, rate: f64| (frame as f64 * 440.0 * std::f64::consts::TAU / rate).sin();
        let input: Vec<f32> = (0..44_100)
            .flat_map(|frame| [wave(frame, 44_100.0) as f32, 0.0])
            .collect();
        for resampling in [
            SoxrSettings::default(),
            soxr_settings(|settings| settings.quality = soxr::Quality::High.into()),
        ] {
            let mut converter =
                Converter::new(spec(44_100), spec(48_000), Resampling::Soxr(resampling)).unwrap();
            let mut out = Vec::new();
            for chunk in input.chunks(2 * 333) {
                converter.process(chunk, &mut out).unwrap();
            }
            converter.finish(&mut out).unwrap();

            let frames = out.as_chunks::<2>().0.iter().enumerate();
            for (frame, samples) in frames.take(47_000).skip(2_000) {
                let error = (f64::from(samples[0]) - wave(frame, 48_000.0)).abs();
                assert!(error < 1e-5, "frame {frame} off by {error}");
                assert_eq!(samples[1], 0.0, "frame {frame}");
            }
        }
    }

    fn soxr_settings(change: impl FnOnce(&mut SoxrSettings)) -> SoxrSettings {
        let mut settings = SoxrSettings::default();
        change(&mut settings);
        settings
    }

    #[test]
    fn every_soxr_option_keeps_the_length_and_the_level() {
        let spec = |rate| Spec { rate, channels: 2 };
        let changes: [fn(&mut SoxrSettings); 10] = [
            |settings| settings.quality = soxr::Quality::Low.into(),
            |settings| settings.quality = soxr::Quality::High.into(),
            |settings| settings.quality.precision = 33.0,
            |settings| settings.quality.passband_end = 0.998,
            |settings| settings.quality.phase_response = soxr::Phase::Minimum.response(),
            |settings| {
                settings.quality = soxr::Quality::High.into();
                settings.quality.double_precision = true;
            },
            |settings| settings.runtime.coef_size_kbytes = 1,
            |settings| settings.runtime.coef_interpolation = soxr::CoefInterpolation::High,
            |settings| settings.runtime.log2_min_dft_size = 15,
            |settings| settings.runtime.log2_large_dft_size = 8,
        ];
        for change in changes {
            let settings = soxr_settings(change);
            let resampling = Resampling::Soxr(settings);
            let mut converter = Converter::new(spec(44_100), spec(48_000), resampling).unwrap();
            let input: Vec<f32> = sine(44_100, 44_100)
                .into_iter()
                .flat_map(|sample| [sample, -sample])
                .collect();
            let mut out = Vec::new();
            for chunk in input.chunks(2 * 333) {
                converter.process(chunk, &mut out).unwrap();
            }
            converter.finish(&mut out).unwrap();

            assert_eq!(out.len(), 2 * 48_000, "{settings:?}");
            let middle = &out[2 * 20_000..2 * 28_000];
            let peak = middle
                .iter()
                .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
            assert!((peak - 1.0).abs() < 0.01, "{settings:?}: {peak}");
        }
    }

    #[test]
    fn a_setting_outside_the_range_of_soxr_is_an_error() {
        let spec = |rate| Spec { rate, channels: 2 };
        let settings = soxr_settings(|settings| settings.quality.precision = 10.0);
        assert!(Converter::new(spec(44_100), spec(48_000), Resampling::Soxr(settings)).is_err());
    }

    /// How the input is split does not change a bit of soxr's output.
    #[test]
    fn soxr_starts_over_on_a_reset_and_takes_any_split() {
        let spec = |rate| Spec { rate, channels: 1 };
        let resampling = Resampling::Soxr(SoxrSettings::default());
        let mut converter = Converter::new(spec(44_100), spec(48_000), resampling).unwrap();
        let input = sine(44_100, 8_000);
        let mut first = Vec::new();
        converter.process(&input, &mut first).unwrap();
        assert!(converter.undelivered_frames() > 0);

        converter.reset().unwrap();
        let mut again = Vec::new();
        for chunk in input.chunks(97) {
            converter.process(chunk, &mut again).unwrap();
        }
        assert_eq!(first, again);
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
        for resampling in each_resampling() {
            let mut converter = Converter::new(source, output, resampling).unwrap();
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
        }

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
        for resampling in each_resampling() {
            let mut converter = Converter::new(source, output, resampling).unwrap();
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
    }

    /// Ratios whose FFT block is larger than what one call pushes return
    /// nothing for several calls; the tail must still come out whole.
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
        for resampling in each_resampling() {
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
                    let mut converter = Converter::new(source, output, resampling).unwrap();
                    let mut out = Vec::new();
                    for chunk in vec![0.25; frames * 2].chunks(2 * 333) {
                        converter.process(chunk, &mut out).unwrap();
                    }
                    converter.finish(&mut out).unwrap();
                    let expected =
                        (frames as f64 * f64::from(to) / f64::from(from)).round() as usize;
                    assert_eq!(
                        out.len() / 2,
                        expected,
                        "{resampling:?}: {from} -> {to} Hz, {frames} frames"
                    );
                }
            }
        }
    }

    #[test]
    fn the_fft_size_reaches_the_resampler() {
        let fft_size_in = |fft_size| {
            let resampling = Resampling::new(fft_size, ResamplingWindow::default());
            fft_size_in(&RateConverter::new(44_100, 48_000, 2, resampling).unwrap())
        };
        assert_eq!(
            [256, 1024, 4096, 16_384].map(fft_size_in),
            [294, 1029, 4116, 16_464]
        );
    }

    #[test]
    fn a_rate_of_0_is_an_error() {
        for (from, to) in [(0, 48_000), (44_100, 0)] {
            assert!(RateConverter::new(from, to, 2, Resampling::default()).is_err());
        }
    }

    #[test]
    fn a_large_fall_in_rate_keeps_the_treble() {
        let tone = 20_000.0;
        for (from, to) in [(96_000, 44_100), (192_000, 48_000), (192_000, 44_100)] {
            let source = Spec {
                rate: from,
                channels: 1,
            };
            let output = Spec {
                rate: to,
                channels: 1,
            };
            let input: Vec<f32> = (0..from)
                .map(|frame| (frame as f64 * tone * std::f64::consts::TAU / f64::from(from)).sin())
                .map(|sample| sample as f32)
                .collect();
            let mut converter = Converter::new(source, output, Resampling::default()).unwrap();
            let mut out = Vec::new();
            converter.process(&input, &mut out).unwrap();
            converter.finish(&mut out).unwrap();
            let middle = &out[out.len() / 4..out.len() * 3 / 4];
            let power = middle
                .iter()
                .map(|sample| f64::from(*sample).powi(2))
                .sum::<f64>()
                / middle.len() as f64;
            let level = 10.0 * (2.0 * power).log10();
            assert!(level.abs() < 0.1, "{from} -> {to} Hz: {level:.2} dB");
        }
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
        assert_eq!(fft_size_in(resampler), 4116);
    }

    #[test]
    fn samples_that_are_no_numbers_do_not_stop_the_resampler() {
        let spec = |rate| Spec { rate, channels: 2 };
        for resampling in each_resampling() {
            let mut converter = Converter::new(spec(44_100), spec(48_000), resampling).unwrap();
            let mut input = vec![0.25; 2 * 4_410];
            input[1_000] = f32::NAN;
            input[1_001] = f32::INFINITY;
            let mut out = Vec::new();
            converter.process(&input, &mut out).unwrap();
            converter.finish(&mut out).unwrap();
            assert_eq!(out.len() / 2, 4_800);
            assert!(out.iter().all(|sample| sample.is_finite()));
        }
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
        for resampling in each_resampling() {
            let mut converter = Converter::new(source, output, resampling).unwrap();
            let mut out = Vec::new();
            converter.process(&vec![0.25; 2 * 100], &mut out).unwrap();
            converter.finish(&mut out).unwrap();
            assert_eq!(out.len() / 2, 92);
        }
    }

    const fn stereo(rate: u32) -> Spec {
        Spec { rate, channels: 2 }
    }

    fn crossfeed() -> dsp::Settings {
        dsp::Settings::new(dsp::Crossfeed::new(700, 4.5), dsp::Balance::CENTER)
    }

    fn holding(settings: dsp::Settings) -> Arc<AtomicU64> {
        Arc::new(AtomicU64::new(settings.to_bits()))
    }

    /// One second of a full-scale 50 Hz sine on the left only
    fn left_tone(rate: u32) -> Vec<f32> {
        (0..rate)
            .flat_map(|frame| {
                let at = f64::from(frame) * 50.0 * std::f64::consts::TAU / f64::from(rate);
                [at.sin() as f32, 0.0]
            })
            .collect()
    }

    /// The level of 50 Hz in each of two interleaved channels, in dB against
    /// `full`, over the half second from a quarter of a second in.
    fn levels_at_50_hz(channels: [Vec<f64>; 2], rate: u32, full: f64) -> [f64; 2] {
        let start = rate as usize / 4;
        let len = rate as usize / 2;
        channels.map(|channel| {
            let (mut re, mut im) = (0.0, 0.0);
            for (n, sample) in channel[start..start + len].iter().enumerate() {
                let (sin, cos) =
                    (std::f64::consts::TAU * 50.0 * n as f64 / f64::from(rate)).sin_cos();
                re += sample * cos;
                im += sample * sin;
            }
            20.0 * (2.0 * re.hypot(im) / len as f64 / full).log10()
        })
    }

    fn split(samples: &[f32]) -> [Vec<f64>; 2] {
        [0, 1].map(|channel| {
            samples
                .iter()
                .skip(channel)
                .step_by(2)
                .map(|sample| f64::from(*sample))
                .collect()
        })
    }

    #[test]
    fn crossfeed_feeds_the_left_into_the_right_with_each_resampler() {
        let plain = OutputSettings::default();
        let cases = std::iter::once((48_000, plain.clone())).chain(
            each_resampling()
                .into_iter()
                .map(|resampling| (44_100, plain.clone().with_resampling(resampling))),
        );
        for (source_rate, settings) in cases {
            let mut converter = Converter::for_settings(
                stereo(source_rate),
                stereo(48_000),
                &settings,
                &holding(crossfeed()),
            )
            .unwrap();
            let mut out = Vec::new();
            converter
                .process(&left_tone(source_rate), &mut out)
                .unwrap();
            let [left, right] = levels_at_50_hz(split(&out), 48_000, 1.0);
            assert!((right - left + 4.5).abs() < 0.2, "{left} {right}");
        }
    }

    #[test]
    fn crossfeed_feeds_the_left_into_the_right_before_the_dsd_encoder() {
        let dop = dsd::Settings::new(
            dsd::DopRate::Dsd64,
            dsd::Filter::default(),
            dsd::Modulation::default(),
        );
        let settings = OutputSettings::new(Some("dac".to_owned()), None).with_dop(Some(dop));
        let frame_rate = 176_400;
        for (dsp, fed) in [(crossfeed(), true), (dsp::Settings::default(), false)] {
            let mut converter = Converter::for_settings(
                stereo(44_100),
                stereo(frame_rate),
                &settings,
                &holding(dsp),
            )
            .unwrap();
            let mut out = Vec::new();
            converter.process(&left_tone(44_100), &mut out).unwrap();
            let bits = split(&out).map(|words| {
                words
                    .iter()
                    .flat_map(|word| {
                        let word = *word as u16;
                        (0..16)
                            .rev()
                            .map(move |bit| if word >> bit & 1 == 1 { 1.0 } else { -1.0 })
                    })
                    .collect()
            });
            let [left, right] = levels_at_50_hz(bits, frame_rate * 16, 0.5);
            if fed {
                assert!((right - left + 4.5).abs() < 0.3, "{left} {right}");
            } else {
                assert!(right - left < -60.0, "{left} {right}");
            }
        }
    }

    #[test]
    fn neutral_settings_leave_the_samples_as_they_are() {
        let dsp = holding(crossfeed());
        let settings = OutputSettings::default();
        let mut converter =
            Converter::for_settings(stereo(48_000), stereo(48_000), &settings, &dsp).unwrap();
        converter
            .process(&left_tone(48_000), &mut Vec::new())
            .unwrap();
        dsp.store(0, Ordering::Relaxed);
        converter
            .process(&left_tone(48_000), &mut Vec::new())
            .unwrap();
        let input: Vec<f32> = (0..9_600).map(|n| (n as f32 * 0.37).sin()).collect();
        let mut out = Vec::new();
        converter.process(&input, &mut out).unwrap();
        let bits = |samples: &[f32]| {
            samples
                .iter()
                .map(|sample| sample.to_bits())
                .collect::<Vec<_>>()
        };
        assert_eq!(bits(&out), bits(&input));
    }

    #[test]
    fn new_settings_apply_from_the_next_packet() {
        let dsp = holding(dsp::Settings::default());
        let settings = OutputSettings::default();
        let mut converter =
            Converter::for_settings(stereo(48_000), stereo(48_000), &settings, &dsp).unwrap();
        let mut out = Vec::new();
        converter.process(&[1.0; 200], &mut out).unwrap();
        assert_eq!(out, [1.0; 200]);
        dsp.store(
            dsp::Settings::new(None, dsp::Balance::from_db(-6.0)).to_bits(),
            Ordering::Relaxed,
        );
        out.clear();
        converter.process(&[1.0; 4_000], &mut out).unwrap();
        let lowered = 10f64.powf(-6.0 / 20.0) as f32;
        assert!(out[1] < 1.0 && out[1] > lowered);
        // 20 ms at 48 kHz
        assert!(
            out[2 * 959..]
                .iter()
                .skip(1)
                .step_by(2)
                .all(|right| *right == lowered)
        );
        assert!(out.iter().step_by(2).all(|left| *left == 1.0));
    }

    #[test]
    fn the_next_track_follows_the_same_settings() {
        let dsp = holding(dsp::Settings::default());
        let settings = OutputSettings::default();
        let converter =
            Converter::for_settings(stereo(48_000), stereo(48_000), &settings, &dsp).unwrap();
        let mut next = converter.following(stereo(48_000), &settings).unwrap();
        dsp.store(
            dsp::Settings::new(None, dsp::Balance::from_db(-6.0)).to_bits(),
            Ordering::Relaxed,
        );
        let mut out = Vec::new();
        next.process(&[1.0; 4_000], &mut out).unwrap();
        let lowered = 10f64.powf(-6.0 / 20.0) as f32;
        assert_eq!(out[out.len() - 2..], [1.0, lowered]);
    }

    #[test]
    fn the_next_track_goes_on_from_where_the_processor_is() {
        let settings = OutputSettings::default();
        let mut converter = Converter::for_settings(
            stereo(44_100),
            stereo(44_100),
            &settings,
            &holding(crossfeed()),
        )
        .unwrap();
        let mut out = Vec::new();
        converter
            .process(&[0.5, 0.0].repeat(44_100), &mut out)
            .unwrap();
        let steady = [out[out.len() - 2], out[out.len() - 1]];
        let mut next = converter.following(stereo(44_100), &settings).unwrap();
        let mut first = Vec::new();
        next.process(&[0.5, 0.0], &mut first).unwrap();
        assert!((first[0] - steady[0]).abs() < 1e-6, "{first:?} {steady:?}");
        assert!((first[1] - steady[1]).abs() < 1e-6, "{first:?} {steady:?}");
    }
}
