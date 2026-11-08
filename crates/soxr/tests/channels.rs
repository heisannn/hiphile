//! Channels: each on its own, in either layout, on any number of threads

mod common;

use common::{Random, noise};
use soxr::{Quality, QualitySpec, Recipe, Resampler, RuntimeSpec};

fn same_bits(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.to_bits() == b.to_bits())
}

fn interleave(channels: &[Vec<f32>]) -> Vec<f32> {
    (0..channels[0].len())
        .flat_map(|frame| channels.iter().map(move |channel| channel[frame]))
        .collect()
}

#[test]
fn split_and_interleaved_buffers_give_the_same_bits() {
    let channels: Vec<Vec<f32>> = (0..3).map(|seed| noise(5000, seed + 1)).collect();
    let interleaved = interleave(&channels);
    let mut resampler = Resampler::<f32, f32>::new(44_100.0, 48_000.0, 3).unwrap();
    let mut output = vec![0.0f32; 6000 * 3];
    let progress = resampler
        .resample(&interleaved[..], &mut output[..])
        .unwrap();
    let rest = resampler
        .finish(&mut output[progress.output_frames * 3..])
        .unwrap();
    output.truncate((progress.output_frames + rest) * 3);
    assert_eq!(output.len(), 5442 * 3);

    let mut resampler = Resampler::<f32, f32>::new(44_100.0, 48_000.0, 3).unwrap();
    let inputs: Vec<&[f32]> = channels.iter().map(Vec::as_slice).collect();
    let mut buffers = vec![vec![0.0f32; 6000]; 3];
    let mut room: Vec<&mut [f32]> = buffers.iter_mut().map(Vec::as_mut_slice).collect();
    let progress = resampler.resample(&inputs[..], &mut room[..]).unwrap();
    let mut rest: Vec<&mut [f32]> = buffers
        .iter_mut()
        .map(|buffer| &mut buffer[progress.output_frames..])
        .collect();
    let left = resampler.finish(&mut rest[..]).unwrap();
    for buffer in &mut buffers {
        buffer.truncate(progress.output_frames + left);
    }
    assert!(same_bits(&interleave(&buffers), &output));
}

#[test]
fn a_channel_comes_out_as_it_would_alone() {
    let channels: Vec<Vec<f32>> = (0..3).map(|seed| noise(5000, seed + 1)).collect();
    for quality in [Quality::High, Quality::VeryHigh, Quality::Quick] {
        let together: Vec<f32> = Resampler::<f32, f32>::builder(44_100.0, 48_000.0, 3)
            .quality(quality)
            .oneshot(&interleave(&channels))
            .unwrap();
        for (index, channel) in channels.iter().enumerate() {
            let alone: Vec<f32> = Resampler::<f32, f32>::builder(44_100.0, 48_000.0, 1)
                .quality(quality)
                .oneshot(channel)
                .unwrap();
            let picked: Vec<f32> = together.iter().skip(index).step_by(3).copied().collect();
            assert!(same_bits(&picked, &alone), "{quality:?} {index}");
        }
    }
}

/// The output in calls of up to `most` frames each way
fn in_calls(resampler: &mut Resampler, input: &[f32], seed: u64, most: usize) -> Vec<f32> {
    common::streamed(resampler, input, seed, most, most)
}

#[test]
fn threads_change_no_bit() {
    let channels = 5;
    let input: Vec<f32> = noise(120_000 * channels, 6);
    let mut variable = QualitySpec::new(Recipe::new(Quality::High));
    variable.variable_rate = true;
    let qualities = [
        QualitySpec::new(Recipe::new(Quality::High)),
        QualitySpec::new(Recipe::new(Quality::VeryHigh).steep(true)),
        QualitySpec::new(Recipe::new(Quality::Bits32)),
        QualitySpec::new(Recipe::new(Quality::Quick)),
        variable,
    ];
    for quality in qualities {
        for rates in [(44_100.0, 48_000.0), (96_000.0, 44_100.0)] {
            let run = |threads: usize, seed: u64| {
                let mut resampler = Resampler::<f32, f32>::builder(rates.0, rates.1, channels)
                    .quality(quality)
                    .runtime(RuntimeSpec::new(threads))
                    .build()
                    .unwrap();
                assert!(resampler.threads() >= 1);
                in_calls(&mut resampler, &input, seed, 30_000)
            };
            let serial = run(1, 1);
            for (threads, seed) in [(2, 1), (3, 2), (0, 3), (8, 4)] {
                assert!(
                    same_bits(&run(threads, seed), &serial),
                    "{quality:?} {rates:?} {threads}"
                );
            }
        }
    }
}

#[test]
fn threads_change_no_bit_as_the_ratio_moves() {
    let channels = 4;
    let input: Vec<f32> = noise(100_000 * channels, 7);
    let mut quality = QualitySpec::new(Recipe::new(Quality::High));
    quality.variable_rate = true;
    let run = |threads: usize| {
        let mut resampler = Resampler::<f32, f32>::builder(1.0, 1.0, channels)
            .quality(quality)
            .runtime(RuntimeSpec::new(threads))
            .build()
            .unwrap();
        let mut random = Random::new(3);
        let mut output = Vec::new();
        let mut rest = &input[..];
        let mut changes = [(20_000, 1.7, 5000), (50_000, 0.6, 0), (60_000, 2.5, 20_000)].iter();
        let mut next = changes.next();
        while !rest.is_empty() {
            let written = output.len() / channels;
            if let Some(&(at, io_ratio, slew_len)) = next
                && written >= at
            {
                resampler.set_io_ratio(io_ratio, slew_len).unwrap();
                next = changes.next();
            }
            let until = next.map_or(usize::MAX, |change| change.0 - written);
            let mut room = vec![0.0f32; random.len(30_000).min(until) * channels];
            let offered = random.len(30_000).min(rest.len() / channels);
            let progress = resampler
                .resample(&rest[..offered * channels], &mut room[..])
                .unwrap();
            rest = &rest[progress.input_frames * channels..];
            output.extend_from_slice(&room[..progress.output_frames * channels]);
        }
        let mut room = vec![0.0f32; 400_000 * channels];
        let left = resampler.finish(&mut room[..]).unwrap();
        output.extend_from_slice(&room[..left * channels]);
        output
    };
    let serial = run(1);
    assert!(same_bits(&run(4), &serial));
}
