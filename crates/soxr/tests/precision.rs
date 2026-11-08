//! A linear chirp from 0 Hz, resampled and compared with the same chirp made
//! at the output rate

use std::f64::consts::PI;

use soxr::{Quality, QualitySpec, Resampler, Rolloff};

const SECONDS: f64 = 8.0;
const MARGIN_SECONDS: f64 = 0.5;
const AMPLITUDE: f64 = 0.5;

fn chirp(rate: f64, top: f64) -> impl Fn(f64) -> f64 {
    move |index: f64| {
        let time = index / rate;
        AMPLITUDE * (PI * top * time * time / SECONDS).sin()
    }
}

/// `-log2` of the largest error, away from the ends
fn bits(quality: Quality, rolloff: Rolloff, top_fraction: f64, rates: (f64, f64)) -> f64 {
    let (input_rate, output_rate) = rates;
    let top = top_fraction * input_rate.min(output_rate) / 2.0;
    let source = chirp(input_rate, top);
    let input: Vec<f64> = (0..(SECONDS * input_rate) as usize)
        .map(|index| source(index as f64))
        .collect();
    let mut spec = QualitySpec::new(quality);
    spec.rolloff = rolloff;
    spec.high_precision_clock = true;
    let output = Resampler::<f64, f64>::builder(input_rate, output_rate, 1)
        .quality(spec)
        .oneshot(&input[..])
        .unwrap();
    let expected = chirp(output_rate, top);
    let first = (MARGIN_SECONDS * output_rate) as usize;
    let last = ((SECONDS - MARGIN_SECONDS) * output_rate) as usize;
    let error = (first..last)
        .map(|index| (output[index] - expected(index as f64)).abs())
        .fold(0.0, f64::max);
    -error.log2()
}

const RATES: [(f64, f64); 5] = [
    (44_100.0, 48_000.0),
    (48_000.0, 44_100.0),
    (44_100.0, 96_000.0),
    (96_000.0, 44_100.0),
    (44_100.0, 50_000.123),
];

fn check(quality: Quality, least: f64) {
    for rates in RATES {
        for (rolloff, top) in [(Rolloff::Small, 0.5), (Rolloff::None, 0.8)] {
            let bits = bits(quality, rolloff, top, rates);
            println!("{quality:?} {rates:?} {rolloff:?} {top}: {bits:.2}");
            assert!(
                bits >= least,
                "{quality:?} {rates:?} {rolloff:?}: {bits:.2} bits"
            );
        }
    }
}

#[test]
fn high_quality_keeps_20_bits() {
    check(Quality::High, 20.0);
}

#[test]
fn very_high_quality_keeps_29_bits() {
    check(Quality::VeryHigh, 29.0);
}

/// Without libsoxr's flag for a precise clock, time is kept as precisely, so
/// the output is the same
#[test]
fn the_clock_is_as_precise_without_its_flag() {
    let rates = (44_100.0, 50_000.123);
    let input: Vec<f64> = common_chirp(rates.0);
    let with_flag = |flag: bool| {
        let mut spec = QualitySpec::new(Quality::VeryHigh);
        spec.high_precision_clock = flag;
        Resampler::<f64, f64>::builder(rates.0, rates.1, 1)
            .quality(spec)
            .oneshot(&input[..])
            .unwrap()
    };
    assert_eq!(with_flag(false), with_flag(true));
}

fn common_chirp(rate: f64) -> Vec<f64> {
    let source = chirp(rate, 0.5 * rate / 2.0);
    (0..(SECONDS * rate) as usize)
        .map(|index| source(index as f64))
        .collect()
}
