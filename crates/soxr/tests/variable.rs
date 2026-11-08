//! A variable rate

mod common;

use std::f64::consts::TAU;

use common::{Random, streamed};
use soxr::{Engine, Error, Quality, QualitySpec, Recipe, Resampler};

fn variable(quality: Quality) -> QualitySpec {
    let mut spec = QualitySpec::new(Recipe::new(quality));
    spec.variable_rate = true;
    spec
}

fn resampler(io_ratio: f64, channels: usize) -> Resampler<f64, f64> {
    Resampler::builder(io_ratio, 1.0, channels)
        .quality(variable(Quality::High))
        .build()
        .unwrap()
}

/// All the output of `input`, with no change of ratio
fn whole(resampler: &mut Resampler<f64, f64>, input: &[f64]) -> Vec<f64> {
    streamed(resampler, input, 1, input.len().max(1), 1 << 20)
}

fn sine(len: usize, frequency: f64) -> Vec<f64> {
    (0..len)
        .map(|index| (TAU * frequency * index as f64).sin())
        .collect()
}

/// The amplitude of the sine at `frequency` cycles per sample that fits
/// `samples` best, by least squares
fn amplitude(samples: &[f64], frequency: f64) -> f64 {
    let (mut cc, mut ss, mut cs, mut xc, mut xs) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (index, sample) in samples.iter().enumerate() {
        let (sin, cos) = (TAU * frequency * index as f64).sin_cos();
        cc += cos * cos;
        ss += sin * sin;
        cs += cos * sin;
        xc += sample * cos;
        xs += sample * sin;
    }
    let determinant = cc * ss - cs * cs;
    let a = (xc * ss - xs * cs) / determinant;
    let b = (xs * cc - xc * cs) / determinant;
    a.hypot(b)
}

fn middle(samples: &[f64]) -> &[f64] {
    &samples[samples.len() / 4..samples.len() * 3 / 4]
}

/// Changes of the ratio: at an output frame, to a ratio, over a slew
type Changes = [(usize, f64, usize)];

/// The output of `input`, in calls of random lengths, changing the ratio as
/// `changes` says
fn with_changes(
    resampler: &mut Resampler<f64, f64>,
    input: &[f64],
    changes: &Changes,
    seed: u64,
) -> Vec<f64> {
    let channels = resampler.channels();
    let mut random = Random::new(seed);
    let mut output = Vec::new();
    let mut rest = input;
    let mut changes = changes.iter().peekable();
    loop {
        let written = output.len() / channels;
        if let Some(&&(at, io_ratio, slew_len)) = changes.peek()
            && at == written
        {
            resampler.set_io_ratio(io_ratio, slew_len).unwrap();
            changes.next();
            continue;
        }
        let until = changes
            .peek()
            .map_or(usize::MAX, |change| change.0 - written);
        let mut room = vec![0.0; random.len(3000).min(until) * channels];
        let frames = if rest.is_empty() {
            resampler.finish(&mut room[..]).unwrap()
        } else {
            let offered = random.len(4000).min(rest.len() / channels);
            let progress = resampler
                .resample(&rest[..offered * channels], &mut room[..])
                .unwrap();
            rest = &rest[progress.input_frames * channels..];
            progress.output_frames
        };
        if rest.is_empty() && frames == 0 {
            return output;
        }
        output.extend_from_slice(&room[..frames * channels]);
    }
}

/// The times of the output frames, in input frames, as the documentation of
/// `set_io_ratio` gives them
fn times(io_ratio: f64, changes: &Changes, len: usize) -> Vec<f64> {
    let mut times = Vec::with_capacity(len);
    let (mut time, mut ratio) = (0.0, io_ratio);
    let mut slew: Option<(f64, f64, usize, usize)> = None;
    let mut changes = changes.iter().peekable();
    for output in 0..len {
        if let Some(&&(at, to, slew_len)) = changes.peek()
            && at == output
        {
            let from = slew.map_or(ratio, |(from, to, len, done)| {
                from + (to - from) * (done as f64 + 0.5) / len as f64
            });
            slew = (slew_len > 0).then_some((from, to, slew_len, 0));
            ratio = to;
            changes.next();
        }
        times.push(time);
        let step = match &mut slew {
            Some((from, to, len, done)) => {
                let step = *from + (*to - *from) * (*done as f64 + 0.5) / *len as f64;
                *done += 1;
                step
            }
            None => ratio,
        };
        if slew.is_some_and(|(_, _, len, done)| done == len) {
            slew = None;
        }
        time += step;
    }
    times
}

#[test]
fn the_engine_is_variable() {
    assert_eq!(resampler(2.0, 1).engine(), Engine::Variable);
}

#[test]
fn the_ratio_starts_as_created_and_comes_back_on_clearing() {
    let mut resampler = resampler(4.0, 1);
    assert_eq!(whole(&mut resampler, &vec![0.0; 40_000]).len(), 10_000);
    resampler.clear();
    resampler.set_io_ratio(1.0, 0).unwrap();
    assert_eq!(whole(&mut resampler, &vec![0.0; 10_000]).len(), 10_000);
    resampler.clear();
    assert_eq!(resampler.io_ratio(), 4.0);
    assert_eq!(whole(&mut resampler, &vec![0.0; 40_000]).len(), 10_000);
    resampler.clear();
    resampler.set_io_ratio(2.0, 0).unwrap();
    assert_eq!(whole(&mut resampler, &vec![0.0; 20_000]).len(), 10_000);
}

#[test]
fn a_ratio_that_never_changes_ends_as_a_constant_one() {
    for (io_ratio, len) in [(48_000.0 / 44_100.0, 50_000), (0.37, 12_345), (2.5, 777)] {
        let output = whole(&mut resampler(io_ratio, 1), &vec![0.0; len]);
        assert_eq!(
            output.len(),
            (len as f64 / io_ratio + 0.5).floor() as usize,
            "{io_ratio}"
        );
    }
}

/// Output frame k is the input at k times the ratio, at every ratio, with no
/// delay
#[test]
fn every_output_frame_is_on_time() {
    for io_ratio in [1.0f64, 0.5, 2.0, 1.0884, 3.9, 1.0 / 64.0, 64.0] {
        let frequency = 0.1 / io_ratio.max(1.0);
        let input = sine(
            200_000.min((20_000.0 * io_ratio) as usize).max(4000),
            frequency,
        );
        let output = whole(&mut resampler(io_ratio, 1), &input);
        let edge = (100.0 * io_ratio.max(1.0) / io_ratio) as usize;
        let worst = output[edge..output.len() - edge]
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let time = (index + edge) as f64 * io_ratio;
                (value - (TAU * frequency * time).sin()).abs()
            })
            .fold(0.0, f64::max);
        assert!(worst < 1e-6, "{io_ratio}: {worst:e}");
    }
}

/// Flat to 0.9 of the lower Nyquist frequency, which is the middle of the
/// transition at 0.95
#[test]
fn the_pass_band_is_flat_to_nine_tenths() {
    for io_ratio in [0.5f64, 1.0, 2.0, 1.0884, 3.9] {
        let lower = 0.5 / io_ratio.max(1.0);
        for (fraction, least, most) in [
            (0.1, -0.002, 0.002),
            (0.5, -0.002, 0.002),
            (0.9, -0.002, 0.002),
            (0.95, -6.1, -5.9),
        ] {
            let frequency = lower * fraction;
            let output = whole(&mut resampler(io_ratio, 1), &sine(60_000, frequency));
            let gain = 20.0 * amplitude(middle(&output), frequency * io_ratio).log10();
            assert!(
                (least..=most).contains(&gain),
                "{io_ratio} at {fraction}: {gain} dB"
            );
        }
    }
}

/// What lies above the lower Nyquist frequency is 116 dB down or more, as
/// libsoxr's is
#[test]
fn what_would_alias_is_removed() {
    for io_ratio in [1.0884f64, 2.0, 3.9, 64.0] {
        for above in [1.0, 1.02, 1.1, 1.5] {
            let frequency = 0.5 / io_ratio * above;
            if frequency >= 0.5 {
                continue;
            }
            let len = (60_000.0 * io_ratio.min(8.0)) as usize;
            let output = whole(&mut resampler(io_ratio, 1), &sine(len, frequency));
            let rest = middle(&output);
            let power = rest.iter().map(|value| value * value).sum::<f64>() / rest.len() as f64;
            let level = 10.0 * (2.0 * power).log10();
            assert!(level < -116.0, "{io_ratio} at {above}: {level} dB");
        }
    }
}

/// Over a slew the step grows linearly, and its frames move the time by their
/// number times the mean of the two ratios
#[test]
fn a_slew_moves_the_ratio_linearly() {
    let origin = 11_000.0;
    let input: Vec<f64> = (0..30_000)
        .map(|index| (index as f64 - origin) / 1000.0)
        .collect();
    let mut resampler = resampler(1.0, 1);
    let output = with_changes(&mut resampler, &input, &[(10_000, 2.0, 1000)], 3);
    let time = |frame: usize| output[frame] * 1000.0 + origin;
    for part in 0..12 {
        let from = 10_000 + 100 * part;
        let moved = time(from + 100) - time(from);
        let expected = if part < 10 {
            105.0 + 10.0 * part as f64
        } else {
            200.0
        };
        assert!((moved - expected).abs() < 1e-3, "{part}: {moved}");
    }
    assert!((time(11_000) - time(10_000) - 1500.0).abs() < 1e-3);
    assert!((time(10_000) - 10_000.0).abs() < 1e-3);
}

/// However the ratio moves, every output frame is the input at its time, so
/// the output has no clicks
#[test]
fn the_output_follows_the_ratio_without_clicks() {
    let changes = [
        (3000, 1.7, 2000),
        (4000, 0.6, 1500),
        (8000, 0.6, 0),
        (9000, 3.0, 0),
        (12_000, 1.25, 5000),
        (20_000, 0.8, 4000),
    ];
    let frequency = 0.03;
    let input = sine(40_000, frequency);
    let output = with_changes(&mut resampler(1.0, 1), &input, &changes, 5);
    let times = times(1.0, &changes, output.len());
    let worst = output
        .iter()
        .zip(&times)
        .filter(|(_, time)| (300.0..input.len() as f64 - 300.0).contains(*time))
        .map(|(value, time)| (value - (TAU * frequency * time).sin()).abs())
        .fold(0.0, f64::max);
    assert!(worst < 1e-6, "{worst:e}");
    // The last frame lies within the input by at least half of its step
    let end = times.last().unwrap() + 0.8 / 2.0;
    assert!(end <= input.len() as f64 && end + 0.8 > input.len() as f64);
}

/// With the ratio changed at the same output frames
#[test]
fn any_split_gives_the_same_bits_as_the_ratio_moves() {
    let changes = [(1000, 1.3, 777), (1500, 0.45, 0), (5000, 2.2, 3000)];
    let input: Vec<f64> = common::noise(60_000, 9)
        .into_iter()
        .map(f64::from)
        .collect();
    let first = with_changes(&mut resampler(1.0, 2), &input, &changes, 1);
    let second = with_changes(&mut resampler(1.0, 2), &input, &changes, 2);
    assert!(
        first.len() == second.len()
            && first
                .iter()
                .zip(&second)
                .all(|(a, b)| a.to_bits() == b.to_bits())
    );
}

#[test]
fn the_quality_has_no_effect() {
    let input: Vec<f64> = common::noise(20_000, 4)
        .into_iter()
        .map(f64::from)
        .collect();
    let outputs: Vec<Vec<f64>> = [Quality::VeryHigh, Quality::Low, Quality::Quick]
        .map(|quality| {
            let mut resampler = Resampler::<f64, f64>::builder(48_000.0, 44_100.0, 1)
                .quality(variable(quality))
                .build()
                .unwrap();
            whole(&mut resampler, &input)
        })
        .into();
    assert!(outputs.iter().all(|output| output == &outputs[0]));
}

/// Set or created, with as much output as a constant ratio gives
#[test]
fn any_ratio_over_zero() {
    let mut set = resampler(4.0, 1);
    let ratios = [
        1e-3,
        1e-2,
        1.0 / 65.0,
        1.0 / 64.0,
        64.0,
        65.0,
        100.0,
        1e3,
        1e6,
    ];
    for io_ratio in ratios {
        assert_eq!(set.set_io_ratio(io_ratio, 0), Ok(()), "{io_ratio}");
    }
    for io_ratio in [0.0, -1.0, f64::INFINITY, f64::NAN] {
        assert_eq!(
            set.set_io_ratio(io_ratio, 10),
            Err(Error::IoRatioOutOfRange),
            "{io_ratio}"
        );
    }
    assert_eq!(set.io_ratio(), 1e6);
    for input_rate in [1.0 / 64.0, 1.0 / 100.0, 64.0, 100.0, 1000.0] {
        let made = Resampler::<f32, f32>::builder(input_rate, 1.0, 1)
            .quality(variable(Quality::High))
            .build();
        assert!(made.is_ok(), "{input_rate}");
    }
    for (io_ratio, input_frames) in [
        (1e-3, 50),
        (64.0, 20_000),
        (65.0, 20_000),
        (100.0, 20_000),
        (1e3, 200_000),
        (1e6, 1_600_000),
    ] {
        let mut resampler = resampler(4.0, 1);
        resampler.set_io_ratio(io_ratio, 0).unwrap();
        let output = whole(&mut resampler, &vec![0.0; input_frames]);
        let expected = (input_frames as f64 / io_ratio + 0.5).floor() as usize;
        assert_eq!(output.len(), expected, "{io_ratio}");
    }
}

/// The delay is the input still to come out, not libsoxr's 100
#[test]
fn the_delay_is_what_is_held() {
    let mut resampler = resampler(2.0, 1);
    assert_eq!(resampler.delay(), 0.0);
    resampler
        .resample(&[0.5f64][..], &mut [0.0f64; 4][..])
        .unwrap();
    assert_eq!(resampler.delay(), 0.5);
    let mut resampler = Resampler::<f64, f64>::builder(1.0, 1.0, 1)
        .quality(variable(Quality::High))
        .build()
        .unwrap();
    let input = vec![0.25; 5000];
    let mut output = vec![0.0; 5000];
    let progress = resampler.resample(&input[..], &mut output[..]).unwrap();
    assert_eq!(
        resampler.delay(),
        (progress.input_frames - progress.output_frames) as f64
    );
    resampler.set_io_ratio(2.0, 0).unwrap();
    let held = (progress.input_frames - progress.output_frames) as f64 / 2.0;
    assert_eq!(resampler.delay(), held);
    let left = resampler.finish(&mut [0.0f64; 0][..]).unwrap();
    assert_eq!(left, 0);
    assert_eq!(resampler.delay(), (held + 0.5).floor());
}
