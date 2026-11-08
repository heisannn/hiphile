#![allow(dead_code)]

use soxr::{Quality, QualitySpec, Recipe, Resampler, Sample};

/// The qualities at which libsoxr was observed, as recipes
pub const QUALITIES: [Quality; 11] = [
    Quality::Quick,
    Quality::Low,
    Quality::Medium,
    Quality::Bits16,
    Quality::High,
    Quality::Bits24,
    Quality::VeryHigh,
    Quality::Bits32,
    Quality::LsrBest,
    Quality::LsrMedium,
    Quality::LsrFast,
];

/// The ratios at which libsoxr was observed
pub const RATES: [(f64, f64); 7] = [
    (44_100.0, 48_000.0),
    (48_000.0, 44_100.0),
    (44_100.0, 96_000.0),
    (96_000.0, 44_100.0),
    (1.0, 2.0),
    (2.0, 1.0),
    (44_100.0, 192_000.0),
];

/// xorshift64*, for reproducible noise and lengths
pub struct Random(u64);

impl Random {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// In `[-1, 1)`
    pub fn sample(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 52) as f64 - 1.0
    }

    /// In `1..=most`
    pub fn len(&mut self, most: usize) -> usize {
        (self.next_u64() % most as u64) as usize + 1
    }
}

pub fn noise(len: usize, seed: u64) -> Vec<f32> {
    let mut random = Random::new(seed);
    (0..len).map(|_| random.sample() as f32).collect()
}

/// The whole output, given input and room in random lengths up to the most
/// given, interleaved with `channels` channels
pub fn streamed<I: Sample, O: Sample>(
    resampler: &mut Resampler<I, O>,
    input: &[I],
    seed: u64,
    most_input: usize,
    most_output: usize,
) -> Vec<O> {
    let channels = resampler.channels();
    let mut random = Random::new(seed);
    let mut output = Vec::new();
    let mut rest = input;
    while !rest.is_empty() {
        let offered = random.len(most_input).min(rest.len() / channels);
        let mut room = vec![O::default(); random.len(most_output) * channels];
        let progress = resampler
            .resample(&rest[..offered * channels], &mut room[..])
            .unwrap();
        rest = &rest[progress.input_frames * channels..];
        output.extend_from_slice(&room[..progress.output_frames * channels]);
    }
    loop {
        let mut room = vec![O::default(); random.len(most_output) * channels];
        let written = resampler.finish(&mut room[..]).unwrap();
        if written == 0 {
            break;
        }
        output.extend_from_slice(&room[..written * channels]);
    }
    output
}

pub fn oneshot<I: Sample, O: Sample>(
    rates: (f64, f64),
    quality: impl Into<QualitySpec>,
    channels: usize,
    input: &[I],
) -> Vec<O> {
    Resampler::<I, O>::builder(rates.0, rates.1, channels)
        .quality(quality)
        .oneshot(input)
        .unwrap()
}

pub fn spec(quality: Quality) -> QualitySpec {
    QualitySpec::new(Recipe::new(quality))
}

/// As libsoxr computes it, in 64-bit floating point
pub fn total_output(input_frames: usize, rates: (f64, f64)) -> usize {
    let ratio = rates.0 / rates.1;
    (input_frames as f64 / ratio + 0.5).floor() as usize
}
