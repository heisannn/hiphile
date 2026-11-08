//! Types, layouts, rounding, clipping and dither

mod common;

use common::{Random, noise, spec};
use soxr::{Dither, Engine, Input, IoSpec, Output, Quality, Resampler, Sample};

const NO_DITHER: IoSpec = IoSpec::new().with_dither(Dither::None);

/// At a ratio of 1, through a fresh resampler, to the end
fn pass<I: Sample, O: Sample>(input: &[I], io: IoSpec) -> (Vec<O>, u64) {
    let mut resampler = Resampler::<I, O>::builder(1.0, 1.0, 1)
        .io(io)
        .build()
        .unwrap();
    let most = input.len().max(1);
    let output = common::streamed(&mut resampler, input, 1, most, most);
    assert_eq!(output.len(), input.len());
    (output, resampler.clips())
}

/// Rounding to the nearest with ties away from zero, as libsoxr did on arm64,
/// and holding to the range
#[test]
fn float_to_i16_rounds_and_clips() {
    let cases: [(f32, i16, bool); 17] = [
        (0.0, 0, false),
        (0.5, 1, false),
        (1.5, 2, false),
        (2.5, 3, false),
        (0.4, 0, false),
        (0.6, 1, false),
        (-0.5, -1, false),
        (-1.5, -2, false),
        (-2.5, -3, false),
        (32_767.4, 32_767, false),
        (32_768.0, 32_767, true),
        (-32_768.0, -32_768, false),
        (49_152.0, 32_767, true),
        (-49_152.0, -32_768, true),
        (32_767.5, 32_767, true),
        (-32_768.5, -32_768, true),
        (-32_769.0, -32_768, true),
    ];
    let input: Vec<f32> = cases.iter().map(|(lsb, _, _)| lsb / 32_768.0).collect();
    let (output, clips) = pass::<f32, i16>(&input, NO_DITHER);
    for ((lsb, expected, _), got) in cases.iter().zip(&output) {
        assert_eq!(got, expected, "{lsb}");
    }
    let expected_clips = cases.iter().filter(|(_, _, clipped)| *clipped).count();
    assert_eq!(clips, expected_clips as u64);
}

#[test]
fn float_to_i32_rounds_and_clips() {
    let lsb = 1.0 / 2_147_483_648.0;
    let (output, clips) = pass::<f32, i32>(&[0.5 * lsb as f32, 1.0, -1.0, 2.0], NO_DITHER);
    assert_eq!(output, [1, i32::MAX, i32::MIN, i32::MAX]);
    assert_eq!(clips, 2);
    let input = [
        0.5 * lsb,
        1.5 * lsb,
        2.5 * lsb,
        1.0,
        -1.0,
        2_147_483_647.5 * lsb,
    ];
    let (output, clips) = pass::<f64, i32>(&input, NO_DITHER);
    assert_eq!(output, [1, 2, 3, i32::MAX, i32::MIN, i32::MAX]);
    assert_eq!(clips, 2);
}

/// Full scale is 1, 2^31 and 2^15
#[test]
fn types_scale_by_their_full_scale() {
    let input: [i16; 4] = [32_767, -32_768, 1, 16_384];
    let (floats, _) = pass::<i16, f32>(&input, IoSpec::new());
    assert_eq!(floats, input.map(|x| f32::from(x) / 32_768.0));
    let (wide, clips) = pass::<i16, i32>(&input, IoSpec::new());
    assert_eq!(wide, input.map(|x| i32::from(x) * 65_536));
    assert_eq!(clips, 0);
    let (doubled, _) = pass::<i16, f64>(&input, IoSpec::new().with_scale(2.0));
    assert_eq!(doubled, input.map(|x| 2.0 * f64::from(x) / 32_768.0));
    let (narrow, _) = pass::<i32, i16>(&[65_536, -131_072, 98_304], NO_DITHER);
    assert_eq!(narrow, [1, -2, 2]);
}

/// Floating-point output is not held to ±1
#[test]
fn floats_are_not_held() {
    let input: Vec<f32> = (0..20_000)
        .map(|index| 1.5 * (index as f32 * 0.01).sin())
        .collect();
    let output: Vec<f32> = common::oneshot((1.0, 2.0), spec(Quality::High), 1, &input);
    let peak = output.iter().fold(0.0f32, |peak, x| peak.max(x.abs()));
    assert!((peak - 1.5).abs() < 1e-3, "{peak}");
}

/// Clips over all channels, until reset or clear
#[test]
fn clips_are_counted_until_reset() {
    let input: Vec<f32> = (0..2000)
        .flat_map(|index| {
            let x = 1.2 * (index as f32 * 0.05).sin();
            [x, x]
        })
        .collect();
    let mut resampler = Resampler::<f32, i16>::builder(1.0, 2.0, 2)
        .io(NO_DITHER)
        .build()
        .unwrap();
    let output = common::streamed(&mut resampler, &input, 3, 500, 700);
    let clipped = output
        .iter()
        .filter(|x| **x == i16::MAX || **x == i16::MIN)
        .count() as u64;
    assert!(clipped > 1000);
    assert_eq!(resampler.clips(), clipped);
    resampler.reset_clips();
    assert_eq!(resampler.clips(), 0);
    let again = common::streamed(&mut resampler, &[], 3, 500, 700);
    assert!(again.is_empty());
    resampler.clear();
    let output = common::streamed(&mut resampler, &input, 4, 300, 900);
    assert_eq!(resampler.clips(), clipped);
    resampler.clear();
    assert_eq!(resampler.clips(), 0);
    assert_eq!(output.len(), 8000);
}

/// TPDF of ±1 LSB, with an error of 1/4 LSB² and no bias
#[test]
fn dither_is_triangular() {
    for level in [0.0f64, 0.25, 0.5, 100.3] {
        let input = vec![level / 32_768.0; 100_000];
        let (output, _) = pass::<f64, i16>(&input, IoSpec::new());
        let errors: Vec<f64> = output.iter().map(|x| f64::from(*x) - level).collect();
        let mean = errors.iter().sum::<f64>() / errors.len() as f64;
        let variance = errors.iter().map(|e| (e - mean) * (e - mean)).sum::<f64>()
            / errors.len() as f64
            + mean * mean;
        assert!(mean.abs() < 0.02, "{level}: mean {mean}");
        assert!((variance - 0.25).abs() < 0.02, "{level}: {variance}");
        if level == 0.0 {
            let share = |value: i16| {
                output.iter().filter(|x| **x == value).count() as f64 / output.len() as f64
            };
            assert!((share(0) - 0.75).abs() < 0.01, "{}", share(0));
            assert!((share(1) - 0.125).abs() < 0.01 && (share(-1) - 0.125).abs() < 0.01);
            assert_eq!(output.iter().filter(|x| x.abs() > 1).count(), 0);
        }
    }
}

/// Dither even at a ratio of 1 from `i16`, but not for other output types
#[test]
fn dither_applies_to_i16_output_only() {
    let input: Vec<i16> = (0..1000).map(|index| (index % 200 - 100) as i16).collect();
    let (output, _) = pass::<i16, i16>(&input, IoSpec::new());
    assert_ne!(output, input);
    assert!(output.iter().zip(&input).all(|(a, b)| (a - b).abs() <= 1));
    let (undithered, _) = pass::<i16, i16>(&input, NO_DITHER);
    assert_eq!(undithered, input);
    let (wide, _) = pass::<i16, i32>(&input, IoSpec::new());
    assert_eq!(
        wide,
        input
            .iter()
            .map(|x| i32::from(*x) << 16)
            .collect::<Vec<_>>()
    );
}

/// Each resampler draws its own noise, and after a clear every one draws the
/// same, as in libsoxr, unless given a seed to start from
#[test]
fn dither_is_the_same_after_clearing_and_follows_its_seed() {
    let input = vec![0.3f64 / 32_768.0; 4000];
    let run = |resampler: &mut Resampler<f64, i16>| {
        let mut output = vec![0i16; input.len()];
        resampler.resample(&input[..], &mut output[..]).unwrap();
        output
    };
    let make = |seed: Option<u64>| {
        Resampler::<f64, i16>::builder(1.0, 1.0, 1)
            .io(IoSpec::new().with_dither(Dither::Tpdf { seed }))
            .build()
            .unwrap()
    };
    let (mut first, mut second) = (make(None), make(None));
    let first_output = run(&mut first);
    assert_ne!(first_output, run(&mut second));
    first.clear();
    second.clear();
    let cleared = run(&mut first);
    assert_ne!(cleared, first_output);
    assert_eq!(run(&mut second), cleared);
    first.clear();
    assert_eq!(run(&mut first), cleared);
    let (mut seeded, mut same) = (make(Some(7)), make(Some(7)));
    let seeded_output = run(&mut seeded);
    assert_eq!(run(&mut same), seeded_output);
    seeded.clear();
    assert_eq!(run(&mut seeded), seeded_output);
    assert_ne!(run(&mut make(Some(8))), seeded_output);
}

/// Dither follows each channel, so a channel comes out as it would alone and
/// the split into calls changes nothing
#[test]
fn dither_is_per_channel_and_split_free() {
    let mono: Vec<f32> = noise(30_000, 6).iter().map(|x| x * 0.001).collect();
    let stereo: Vec<f32> = mono.iter().flat_map(|x| [*x, -*x]).collect();
    let io = IoSpec::new().with_dither(Dither::Tpdf { seed: Some(42) });
    let make = |channels| {
        Resampler::<f32, i16>::builder(44_100.0, 48_000.0, channels)
            .io(io)
            .build()
            .unwrap()
    };
    let alone = common::streamed(&mut make(1), &mono, 1, 4000, 4000);
    let together = common::streamed(&mut make(2), &stereo, 2, 1, 9);
    let left: Vec<i16> = together.iter().step_by(2).copied().collect();
    assert_eq!(left, alone);
    let whole = common::streamed(&mut make(2), &stereo, 3, 30_000, 40_000);
    assert_eq!(whole, together);
}

const LAYOUT_RATES: (f64, f64) = (44_100.0, 48_000.0);

/// Every pairing of interleaved and split buffers gives the same samples
#[test]
fn layouts_give_the_same_output() {
    let channels = 3;
    let frames = 5000;
    let mut random = Random::new(17);
    let split: Vec<Vec<f32>> = (0..channels)
        .map(|_| (0..frames).map(|_| random.sample() as f32).collect())
        .collect();
    let interleaved: Vec<f32> = (0..frames)
        .flat_map(|frame| split.iter().map(move |channel| channel[frame]))
        .collect();
    let channel_slices: Vec<&[f32]> = split.iter().map(Vec::as_slice).collect();
    let total = common::total_output(frames, LAYOUT_RATES);
    let expected = to_end(Input::Interleaved(&interleaved), channels, total, false);
    assert_eq!(expected.len(), total * channels);
    for (input, split_output) in [
        (Input::Interleaved(&interleaved), true),
        (Input::Split(&channel_slices), false),
        (Input::Split(&channel_slices), true),
    ] {
        assert_eq!(to_end(input, channels, total, split_output), expected);
    }
}

/// All of `input` and then the end, as interleaved samples
fn to_end(input: Input<'_, f32>, channels: usize, total: usize, split_output: bool) -> Vec<f64> {
    let mut resampler =
        Resampler::<f32, f64>::new(LAYOUT_RATES.0, LAYOUT_RATES.1, channels).unwrap();
    if !split_output {
        let mut output = vec![0.0; total * channels];
        let progress = resampler.resample(input, &mut output[..]).unwrap();
        let done = progress.output_frames;
        let rest = resampler.finish(&mut output[done * channels..]).unwrap();
        assert_eq!(done + rest, total);
        return output;
    }
    let mut buffers = vec![vec![0.0; total]; channels];
    let mut outputs: Vec<&mut [f64]> = buffers.iter_mut().map(Vec::as_mut_slice).collect();
    let done = resampler
        .resample(input, Output::Split(&mut outputs))
        .unwrap()
        .output_frames;
    let mut rest: Vec<&mut [f64]> = buffers
        .iter_mut()
        .map(|buffer| &mut buffer[done..])
        .collect();
    assert_eq!(resampler.finish(Output::Split(&mut rest)), Ok(total - done));
    (0..total)
        .flat_map(|frame| buffers.iter().map(move |channel| channel[frame]))
        .collect()
}

/// Up to 20 bits of precision the arithmetic is single precision, which shows
/// in `i32` output, unless double precision is asked for
#[test]
fn double_precision_reaches_i32_output() {
    let input: Vec<f64> = noise(20_000, 9)
        .iter()
        .map(|x| f64::from(*x) * 0.5)
        .collect();
    let convert = |double_precision: bool| {
        let mut quality = spec(Quality::High);
        quality.double_precision = double_precision;
        let resampler = Resampler::<f64, i32>::builder(44_100.0, 48_000.0, 1)
            .quality(quality)
            .build()
            .unwrap();
        let engine = resampler.engine();
        let output = Resampler::<f64, i32>::builder(44_100.0, 48_000.0, 1)
            .quality(quality)
            .oneshot(&input[..])
            .unwrap();
        (engine, output)
    };
    let (single_engine, single) = convert(false);
    let (double_engine, double) = convert(true);
    assert_eq!(
        (single_engine, double_engine),
        (Engine::Single, Engine::Double)
    );
    let largest = single
        .iter()
        .zip(&double)
        .map(|(a, b)| (i64::from(*a) - i64::from(*b)).abs())
        .max()
        .unwrap();
    // Single precision leaves errors near 2^-22 of full scale, some hundred
    // LSBs of 32 bits
    assert!((1 << 4..1 << 12).contains(&largest), "{largest}");
}
