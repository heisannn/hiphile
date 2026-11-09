use std::f64::consts::PI;

use super::*;
use crate::{Balance, Crossfeed};

const RATE: u32 = 48_000;
/// 20 ms at [`RATE`]
const TRANSITION: usize = 960;

fn bs2b() -> Option<Crossfeed> {
    Crossfeed::new(700, 4.5)
}

fn jan_meier() -> Option<Crossfeed> {
    Crossfeed::new(650, 9.5)
}

fn processor(settings: Settings) -> Processor {
    Processor::new(RATE, 2, settings)
}

/// A full-scale 1 kHz sine on the left and the same, a radian later, on the right
fn tone(from: usize, frames: usize) -> Vec<f32> {
    (from..from + frames)
        .flat_map(|n| {
            let at = 2.0 * PI * 1000.0 * n as f64 / f64::from(RATE);
            [at.sin() as f32, (at + 1.0).sin() as f32]
        })
        .collect()
}

fn largest_step(samples: &[f32], last: &mut Option<[f32; 2]>) -> f32 {
    let mut largest = 0.0f32;
    for frame in samples.as_chunks::<2>().0 {
        if let Some(last) = last {
            largest = largest
                .max((frame[0] - last[0]).abs())
                .max((frame[1] - last[1]).abs());
        }
        *last = Some(*frame);
    }
    largest
}

fn bits(samples: &[f32]) -> Vec<u32> {
    samples.iter().map(|sample| sample.to_bits()).collect()
}

#[test]
fn neutral_settings_leave_every_sample_as_it_is() {
    let mut samples = vec![0.25, -1.5, f32::NAN, 1e-40, f32::INFINITY, 0.0];
    let before = bits(&samples);
    processor(Settings::default()).process(&mut samples);
    assert_eq!(bits(&samples), before);
}

#[test]
fn moving_back_to_neutral_leaves_samples_as_they_are_again() {
    let mut processor = processor(Settings::new(bs2b(), Balance::from_db(-3.0)));
    processor.process(&mut tone(0, 4800));
    processor.follow(Settings::default());
    processor.process(&mut tone(4800, 4800));
    let mut samples = tone(9600, 4800);
    let before = bits(&samples);
    processor.process(&mut samples);
    assert_eq!(bits(&samples), before);
}

#[test]
fn starting_at_settings_takes_no_transition() {
    let mut samples = vec![1.0; 4];
    processor(Settings::new(None, Balance::from_db(-6.0))).process(&mut samples);
    let lowered = 10f64.powf(-6.0 / 20.0) as f32;
    assert_eq!(samples, [1.0, lowered, 1.0, lowered]);
}

#[test]
fn a_balance_lowers_its_side_over_exactly_20_ms() {
    let mut processor = processor(Settings::default());
    processor.follow(Settings::new(None, Balance::from_db(-6.0)));
    let mut samples = vec![1.0; 2 * (TRANSITION + 10)];
    processor.process(&mut samples);
    let lowered = 10f64.powf(-6.0 / 20.0) as f32;
    let right: Vec<f32> = samples.iter().skip(1).step_by(2).copied().collect();
    assert!(
        right
            .windows(2)
            .all(|pair| pair[1] < pair[0] || pair[1] == lowered)
    );
    assert_ne!(right[TRANSITION - 2], lowered);
    assert!(right[TRANSITION - 1..].iter().all(|gain| *gain == lowered));
    assert!(samples.iter().step_by(2).all(|left| *left == 1.0));
}

#[test]
fn a_balance_changed_part_way_glides_on_from_where_it_is() {
    let mut processor = processor(Settings::default());
    processor.follow(Settings::new(None, Balance::from_db(-12.0)));
    let mut first = vec![1.0; TRANSITION];
    processor.process(&mut first);
    processor.follow(Settings::new(None, Balance::from_db(-3.0)));
    let mut second = vec![1.0; 2];
    processor.process(&mut second);
    assert!((second[1] - first[TRANSITION - 1]).abs() < 0.002);
}

#[test]
fn no_change_steps_harder_than_the_sound_itself() {
    let changes = [
        Settings::new(bs2b(), Balance::from_db(-12.0)),
        Settings::new(jan_meier(), Balance::from_db(4.0)),
        Settings::new(bs2b(), Balance::CENTER),
        Settings::default(),
    ];
    let mut steady = 0.0f32;
    for settings in std::iter::once(Settings::default()).chain(changes) {
        let mut samples = tone(0, 4800);
        processor(settings).process(&mut samples);
        steady = steady.max(largest_step(&samples, &mut None));
    }

    let mut processor = processor(Settings::default());
    let mut last = None;
    let mut largest = 0.0f32;
    let mut at = 0;
    // Every other change comes part-way through the transition to the one before it
    for (settings, frames) in changes.into_iter().zip([300, 4800].into_iter().cycle()) {
        processor.follow(settings);
        let mut samples = tone(at, frames);
        processor.process(&mut samples);
        largest = largest.max(largest_step(&samples, &mut last));
        at += frames;
    }
    let mut samples = tone(at, 4800);
    processor.process(&mut samples);
    largest = largest.max(largest_step(&samples, &mut last));
    assert!(largest <= steady * 1.05, "{largest} against {steady}");
}

#[test]
fn only_the_latest_crossfeed_waits_for_a_crossfade() {
    let mut processor = processor(Settings::default());
    processor.follow(Settings::new(bs2b(), Balance::CENTER));
    processor.process(&mut tone(0, 100));
    processor.follow(Settings::new(Crossfeed::new(700, 6.0), Balance::CENTER));
    processor.follow(Settings::new(jan_meier(), Balance::CENTER));
    processor.process(&mut tone(100, TRANSITION - 100));
    let fade = processor.fade.as_ref().expect("the next crossfade");
    assert_eq!(fade.from.as_ref().map(Filter::crossfeed), bs2b());
    assert_eq!(
        processor.filter.as_ref().map(Filter::crossfeed),
        jan_meier()
    );
    processor.process(&mut tone(TRANSITION, TRANSITION));
    assert!(processor.fade.is_none());
}

#[test]
fn channels_after_the_first_two_are_left_alone() {
    let settings = Settings::new(bs2b(), Balance::from_db(-6.0));
    let mut samples: Vec<f32> = (0..300).map(|n| (n as f32 * 0.01).sin()).collect();
    let before = bits(&samples);
    Processor::new(RATE, 3, settings).process(&mut samples);
    let third = |samples: &[u32]| {
        samples
            .iter()
            .skip(2)
            .step_by(3)
            .copied()
            .collect::<Vec<_>>()
    };
    assert_eq!(third(&bits(&samples)), third(&before));
    assert_ne!(bits(&samples), before);

    let mut mono = samples.clone();
    Processor::new(RATE, 1, settings).process(&mut mono);
    assert_eq!(bits(&mono), bits(&samples));
}

#[test]
fn silence_after_sound_comes_out_as_exact_zeros() {
    let mut processor = processor(Settings::new(bs2b(), Balance::from_db(2.0)));
    processor.process(&mut tone(0, RATE as usize));
    let mut silence = vec![0.0; 2 * RATE as usize];
    for _ in 0..10 {
        silence.fill(0.0);
        processor.process(&mut silence);
    }
    assert!(silence.iter().all(|sample| sample.to_bits() == 0));
}

#[test]
fn a_sample_that_is_not_a_number_counts_as_silence() {
    let mut processor = processor(Settings::new(bs2b(), Balance::CENTER));
    let mut samples = vec![f32::NAN, 0.5, f32::INFINITY, 0.5];
    samples.extend(tone(0, 100));
    processor.process(&mut samples);
    assert!(samples.iter().all(|sample| sample.is_finite()));
}

#[test]
fn at_another_rate_it_goes_on_without_a_step() {
    let mut processor = Processor::new(44_100, 2, Settings::new(bs2b(), Balance::CENTER));
    let mut steady = vec![0.5, 0.0];
    for _ in 0..44_100 {
        steady = vec![0.5, 0.0];
        processor.process(&mut steady);
    }
    processor.follow(Settings::new(bs2b(), Balance::from_db(-6.0)));
    processor.process(&mut [0.5, 0.0].repeat(441));
    let mut processor = processor.at_rate(96_000);
    assert_eq!(processor.gains.frames, 1920);
    assert_eq!(processor.gains.done, 960);
    let mut first = vec![0.5, 0.0];
    processor.process(&mut first);
    assert!((first[0] - steady[0]).abs() < 1e-6);
}
