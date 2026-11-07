use std::time::Duration;

use crate::Error;
use crate::limiter::Limiter;
use crate::modulator::Modulator;
use crate::oversampler::Oversampler;
use crate::settings::Settings;

const BITS_PER_WORD: u32 = 16;
const CHANNELS: usize = 2;

/// Stereo PCM in, interleaved DSD words out, older bits in the higher places.
///
/// One encoder can take several tracks of one source rate in a row with no
/// joint, and [`Self::for_next_track`] keeps the modulators going for another
/// rate.
pub struct Encoder {
    settings: Settings,
    frame_rate: u32,
    oversampler: Oversampler,
    limiter: Option<Limiter>,
    limiter_fill: usize,
    modulators: [Modulator; CHANNELS],
    input_gain: f64,
    fade: Option<Fade>,
    words: [u16; CHANNELS],
    bits: u32,
    converted: Vec<f64>,
    widened: Vec<f64>,
    source_rate: u32,
}

#[derive(Clone, Copy)]
struct Fade {
    done: u64,
    frames: u64,
}

impl Encoder {
    /// `frame_rate` is that of the DoP frames, of [`BITS_PER_WORD`] bits per
    /// channel.
    pub fn new(source_rate: u32, frame_rate: u32, settings: &Settings) -> Result<Self, Error> {
        let bit_rate = frame_rate * BITS_PER_WORD;
        let modulation = &settings.modulation;
        let modulators = [
            Modulator::new(modulation, f64::from(bit_rate), 0x9E37_79B9_7F4A_7C15),
            Modulator::new(modulation, f64::from(bit_rate), 0xD1B5_4A32_D192_ED03),
        ];
        Self::around(source_rate, frame_rate, settings, modulators)
    }

    fn around(
        source_rate: u32,
        frame_rate: u32,
        settings: &Settings,
        modulators: [Modulator; CHANNELS],
    ) -> Result<Self, Error> {
        let bit_rate = frame_rate * BITS_PER_WORD;
        let modulation = &settings.modulation;
        Ok(Self {
            settings: *settings,
            frame_rate,
            oversampler: Oversampler::new(source_rate, bit_rate, settings.filter)?,
            limiter: modulation
                .limiter_ceiling_db
                .map(|ceiling| Limiter::new(ceiling, f64::from(bit_rate))),
            limiter_fill: 0,
            modulators,
            input_gain: 10f64.powf(modulation.input_level_db / 20.0),
            fade: None,
            words: [0; CHANNELS],
            bits: 0,
            converted: Vec::new(),
            widened: Vec::new(),
            source_rate,
        })
    }

    /// An encoder for the track after this one, whose noise shaping and fade
    /// go on where this one's leave off.
    pub fn for_next_track(&self, source_rate: u32) -> Result<Self, Error> {
        let mut next = Self::around(
            source_rate,
            self.frame_rate,
            &self.settings,
            self.modulators.clone(),
        )?;
        next.fade = self.fade;
        Ok(next)
    }

    /// Forgets the audio inside, as after a seek. Noise shaping and a fade go
    /// on.
    pub fn reset(&mut self) -> Result<(), Error> {
        *self = self.for_next_track(self.source_rate)?;
        Ok(())
    }

    /// Raises the level from silence over `length`, starting with the next
    /// input.
    pub fn fade_in(&mut self, length: Duration) {
        let bit_rate = f64::from(self.frame_rate * BITS_PER_WORD);
        self.fade = Some(Fade {
            done: 0,
            frames: (length.as_secs_f64() * bit_rate) as u64,
        });
    }

    pub fn is_fading(&self) -> bool {
        self.fade.is_some()
    }

    /// A sample that is not a finite number counts as silence: it would panic
    /// inside the FFT.
    pub fn push(&mut self, input: &[f32], out: &mut Vec<u16>) -> Result<(), Error> {
        self.widened.clear();
        self.widened.extend(
            input
                .iter()
                .map(|sample| f64::from(*sample))
                .map(|sample| if sample.is_finite() { sample } else { 0.0 }),
        );
        self.converted.clear();
        self.oversampler.push(&self.widened, &mut self.converted)?;
        self.modulate_converted(out)
    }

    /// Frames at the DoP rate that the input so far comes to, still inside.
    pub fn undelivered_frames(&self) -> u64 {
        let held_by_limiter = self.limiter.as_ref().map_or(0, |limiter| {
            limiter.delay_frames().min(self.limiter_fill) as u64
        });
        (self.oversampler.undelivered_frames() + held_by_limiter + u64::from(self.bits))
            / u64::from(BITS_PER_WORD)
    }

    /// Gives up all that is inside and ends on a whole word.
    pub fn finish(&mut self, out: &mut Vec<u16>) -> Result<(), Error> {
        self.converted.clear();
        self.oversampler.finish(&mut self.converted)?;
        self.modulate_converted(out)?;
        self.flush_limiter(out)?;
        while self.bits != 0 {
            self.modulate_frame([0.0, 0.0], out);
        }
        self.fail_if_lost()
    }

    /// Silence pushes out what the limiter holds. The first of it may only
    /// finish filling the delay line, so it takes a whole line's worth.
    fn flush_limiter(&mut self, out: &mut Vec<u16>) -> Result<(), Error> {
        let Some(limiter) = self.limiter.as_ref().filter(|_| self.limiter_fill > 0) else {
            return Ok(());
        };
        self.converted.clear();
        self.converted
            .resize(limiter.delay_frames() * CHANNELS, 0.0);
        self.modulate_converted(out)?;
        self.limiter_fill = 0;
        Ok(())
    }

    fn modulate_converted(&mut self, out: &mut Vec<u16>) -> Result<(), Error> {
        out.reserve(self.converted.len() / BITS_PER_WORD as usize + CHANNELS);
        let converted = std::mem::take(&mut self.converted);
        for frame in converted.as_chunks::<CHANNELS>().0 {
            let mut frame = [frame[0] * self.input_gain, frame[1] * self.input_gain];
            if let Some(fade) = &mut self.fade {
                let gain = fade.done as f64 / fade.frames.max(1) as f64;
                let gain = gain * gain;
                frame = [frame[0] * gain, frame[1] * gain];
                fade.done += 1;
                if fade.done >= fade.frames {
                    self.fade = None;
                }
            }
            if let Some(limiter) = &mut self.limiter {
                let limited = limiter.step(frame);
                if self.limiter_fill < limiter.delay_frames() {
                    self.limiter_fill += 1;
                    continue;
                }
                frame = limited;
            }
            self.modulate_frame(frame, out);
        }
        self.converted = converted;
        self.fail_if_lost()
    }

    #[inline]
    fn modulate_frame(&mut self, frame: [f64; CHANNELS], out: &mut Vec<u16>) {
        for (index, sample) in frame.into_iter().enumerate() {
            let bit = self.modulators[index].step(sample);
            self.words[index] = (self.words[index] << 1) | u16::from(bit);
        }
        self.bits += 1;
        if self.bits == BITS_PER_WORD {
            self.bits = 0;
            out.extend_from_slice(&self.words);
        }
    }

    fn fail_if_lost(&self) -> Result<(), Error> {
        if self.modulators.iter().any(Modulator::is_lost) {
            return Err(Error::Unstable);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{DopRate, Filter, Modulation, ResamplingWindow, SincLength};

    const FRAME_RATE: u32 = 176_400;

    fn settings(modulation: Modulation) -> Settings {
        Settings::new(
            DopRate::Dsd64,
            Filter::Sinc {
                length: SincLength::L256,
                window: ResamplingWindow::BlackmanHarris2,
                cutoff: None,
            },
            modulation,
        )
    }

    fn sine(frames: usize, amplitude: f32) -> Vec<f32> {
        (0..frames)
            .flat_map(|index| {
                let phase = std::f32::consts::TAU * 1_000.0 * index as f32 / 44_100.0;
                let value = amplitude * phase.sin();
                [value, value]
            })
            .collect()
    }

    fn bits_of(words: &[u16], channel: usize) -> Vec<f64> {
        words
            .iter()
            .skip(channel)
            .step_by(2)
            .flat_map(|word| {
                (0..16)
                    .rev()
                    .map(move |shift| if word >> shift & 1 == 1 { 1.0 } else { -1.0 })
            })
            .collect()
    }

    /// The amplitude of 1 kHz in `bits`, at 2.8224 MHz.
    fn level_at_1khz(bits: &[f64]) -> f64 {
        let (mut re, mut im) = (0.0, 0.0);
        for (index, bit) in bits.iter().enumerate() {
            let (sin, cos) =
                (std::f64::consts::TAU * 1_000.0 * index as f64 / 2_822_400.0).sin_cos();
            re += bit * cos;
            im += bit * sin;
        }
        2.0 * re.hypot(im) / bits.len() as f64
    }

    #[test]
    fn words_come_to_the_frames_of_the_input() {
        let mut encoder =
            Encoder::new(44_100, FRAME_RATE, &settings(Modulation::default())).unwrap();
        let mut out = Vec::new();
        for block in sine(44_100, 0.1).chunks(1_000) {
            encoder.push(block, &mut out).unwrap();
        }
        assert!(encoder.undelivered_frames() > 0);
        encoder.finish(&mut out).unwrap();
        assert_eq!(out.len(), FRAME_RATE as usize * 2);
    }

    #[test]
    fn what_it_holds_is_counted_up_to_the_words_it_owes() {
        let mut encoder =
            Encoder::new(44_100, FRAME_RATE, &settings(Modulation::default())).unwrap();
        let mut out = Vec::new();
        encoder.push(&sine(22_050, 0.1), &mut out).unwrap();
        let owed = (FRAME_RATE as usize / 2)
            .abs_diff(out.len() / 2 + encoder.undelivered_frames() as usize);
        assert!(owed <= 1, "{owed}");
    }

    #[test]
    fn half_scale_pcm_is_a_quarter_of_full_modulation() {
        let mut encoder =
            Encoder::new(44_100, FRAME_RATE, &settings(Modulation::default())).unwrap();
        let mut out = Vec::new();
        encoder.push(&sine(44_100, 0.5), &mut out).unwrap();
        encoder.finish(&mut out).unwrap();
        for channel in 0..2 {
            let level = level_at_1khz(&bits_of(&out, channel));
            assert!((level - 0.25).abs() < 1e-3, "{level}");
        }
    }

    #[test]
    fn the_input_level_scales_the_output() {
        let modulation = Modulation::default().with_input_level_db(-6.0206);
        let mut encoder = Encoder::new(44_100, FRAME_RATE, &settings(modulation)).unwrap();
        let mut out = Vec::new();
        encoder.push(&sine(44_100, 0.5), &mut out).unwrap();
        encoder.finish(&mut out).unwrap();
        let level = level_at_1khz(&bits_of(&out, 0));
        assert!((level - 0.125).abs() < 1e-3, "{level}");
    }

    #[test]
    fn the_limiter_keeps_an_overload_stable_and_at_the_ceiling() {
        let mut encoder =
            Encoder::new(44_100, FRAME_RATE, &settings(Modulation::default())).unwrap();
        let mut out = Vec::new();
        encoder.push(&sine(44_100, 3.0), &mut out).unwrap();
        encoder.finish(&mut out).unwrap();
        let ceiling = 0.5 * 10f64.powf(1.0 / 20.0);
        let bits = bits_of(&out, 0);
        let level = level_at_1khz(&bits[bits.len() / 2..]);
        assert!(level < ceiling * 1.01, "{level}");
    }

    #[test]
    fn a_modulator_that_gives_way_ends_it_with_an_error() {
        let modulation = Modulation::default()
            .with_noise_gain(3.0)
            .with_limiter_ceiling_db(None);
        let mut encoder = Encoder::new(44_100, FRAME_RATE, &settings(modulation)).unwrap();
        let mut out = Vec::new();
        let failed = sine(44_100, 1.1)
            .chunks(1_000)
            .any(|block| encoder.push(block, &mut out).is_err());
        assert!(failed);
    }

    #[test]
    fn a_fade_starts_from_silence_and_reaches_the_full_level() {
        let mut encoder =
            Encoder::new(44_100, FRAME_RATE, &settings(Modulation::default())).unwrap();
        encoder.fade_in(Duration::from_millis(500));
        let mut out = Vec::new();
        encoder.push(&sine(66_150, 0.5), &mut out).unwrap();
        encoder.finish(&mut out).unwrap();
        let bits = bits_of(&out, 0);
        let early = level_at_1khz(&bits[..bits.len() / 15]);
        let late = level_at_1khz(&bits[bits.len() * 2 / 3..]);
        assert!(early < 0.02, "{early}");
        assert!((late - 0.25).abs() < 1e-3, "{late}");
    }

    #[test]
    fn a_next_track_keeps_the_modulators_going_at_another_rate() {
        let encoder = Encoder::new(44_100, FRAME_RATE, &settings(Modulation::default())).unwrap();
        let mut next = encoder.for_next_track(88_200).unwrap();
        let mut out = Vec::new();
        next.push(&sine(88_200, 0.1), &mut out).unwrap();
        next.finish(&mut out).unwrap();
        assert_eq!(out.len(), FRAME_RATE as usize * 2);
    }

    #[test]
    fn a_fade_goes_on_into_the_next_track() {
        let mut encoder =
            Encoder::new(44_100, FRAME_RATE, &settings(Modulation::default())).unwrap();
        encoder.fade_in(Duration::from_secs(10));
        let mut next = encoder.for_next_track(88_200).unwrap();
        assert!(next.is_fading());
        let mut out = Vec::new();
        next.push(&sine(88_200, 0.5), &mut out).unwrap();
        next.finish(&mut out).unwrap();
        let level = level_at_1khz(&bits_of(&out, 0));
        assert!(level < 0.01, "{level}");
    }

    #[test]
    fn nothing_is_left_inside_after_the_finish() {
        let mut encoder =
            Encoder::new(44_100, FRAME_RATE, &settings(Modulation::default())).unwrap();
        let mut out = Vec::new();
        encoder.push(&sine(1_000, 0.1), &mut out).unwrap();
        encoder.finish(&mut out).unwrap();
        assert_eq!(encoder.undelivered_frames(), 0);
    }

    #[test]
    fn input_shorter_than_the_limiter_comes_out_whole() {
        for frames in [1, 44, 100, 441, 2_000] {
            let mut encoder =
                Encoder::new(44_100, FRAME_RATE, &settings(Modulation::default())).unwrap();
            let mut out = Vec::new();
            encoder.push(&sine(frames, 0.1), &mut out).unwrap();
            encoder.finish(&mut out).unwrap();
            let expected = frames * FRAME_RATE as usize / 44_100 * 2;
            assert_eq!(out.len(), expected, "{frames} frames");
        }
    }

    #[test]
    fn samples_that_are_no_numbers_count_as_silence() {
        let filters = [
            Filter::Sinc {
                length: SincLength::L256,
                window: ResamplingWindow::BlackmanHarris2,
                cutoff: None,
            },
            Filter::Fft {
                size: 256,
                window: ResamplingWindow::BlackmanHarris2,
            },
        ];
        for filter in filters {
            for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                let settings = Settings::new(DopRate::Dsd64, filter, Modulation::default());
                let mut encoder = Encoder::new(44_100, FRAME_RATE, &settings).unwrap();
                let mut input = sine(4_410, 0.1);
                input[1_000] = bad;
                let mut out = Vec::new();
                encoder.push(&input, &mut out).unwrap();
                encoder.finish(&mut out).unwrap();
                assert_eq!(out.len(), 17_640 * 2);
            }
        }
    }

    #[test]
    fn a_reset_drops_what_is_inside() {
        let mut encoder =
            Encoder::new(44_100, FRAME_RATE, &settings(Modulation::default())).unwrap();
        let mut out = Vec::new();
        encoder.push(&sine(1_000, 0.5), &mut out).unwrap();
        encoder.reset().unwrap();
        assert_eq!(encoder.undelivered_frames(), 0);
    }

    #[test]
    fn every_source_gets_the_standard_rates_unless_it_is_to_match() {
        let standard = settings(Modulation::default());
        assert_eq!(standard.frame_rates_for(44_100), [176_400, 192_000]);
        assert_eq!(standard.frame_rates_for(96_000), [176_400, 192_000]);
        let matching = standard.with_matching_source_family(true);
        assert_eq!(matching.frame_rates_for(44_100), [176_400, 192_000]);
        assert_eq!(matching.frame_rates_for(96_000), [192_000, 176_400]);
    }

    #[test]
    fn a_seek_does_not_end_a_fade() {
        let mut encoder =
            Encoder::new(44_100, FRAME_RATE, &settings(Modulation::default())).unwrap();
        encoder.fade_in(Duration::from_secs(10));
        let mut out = Vec::new();
        encoder.reset().unwrap();
        encoder.push(&sine(44_100, 0.5), &mut out).unwrap();
        encoder.finish(&mut out).unwrap();
        let level = level_at_1khz(&bits_of(&out, 0));
        assert!(level < 0.01, "{level}");
    }

    #[test]
    fn more_than_one_rate_is_offered_per_family() {
        assert_eq!(DopRate::Dsd64.frame_rate_for(44_100), 176_400);
        assert_eq!(DopRate::Dsd64.frame_rate_for(96_000), 192_000);
        assert_eq!(DopRate::Dsd128.frame_rate_for(88_200), 352_800);
        assert_eq!(DopRate::Dsd256.frame_rate_for(48_000), 768_000);
    }
}
