//! The runtime spec

mod common;

use std::f64::consts::TAU;

use common::noise;
use soxr::{CoefInterpolation, Quality, Resampler, RuntimeSpec};

fn with(change: impl FnOnce(&mut RuntimeSpec)) -> Vec<f32> {
    let mut runtime = RuntimeSpec::default();
    change(&mut runtime);
    Resampler::<f32, f32>::builder(44_100.0, 48_000.0, 1)
        .quality(Quality::Low)
        .runtime(runtime)
        .oneshot(&noise(40_000, 3)[..])
        .unwrap()
}

/// No value is an error, as in libsoxr. DFT sizes out of range change
/// nothing, and a table of at most 1 kB interpolates its coefficients.
#[test]
fn values_out_of_range_are_taken() {
    let default = with(|_| {});
    for size in [0, 7, 16, 25] {
        assert_eq!(with(|spec| spec.log2_min_dft_size = size), default);
    }
    for size in [0, 7, 21, 24] {
        assert_eq!(with(|spec| spec.log2_large_dft_size = size), default);
    }
    assert_eq!(with(|spec| spec.coef_size_kbytes = 100_000), default);
    let small = with(|spec| spec.coef_size_kbytes = 0);
    assert_eq!(with(|spec| spec.coef_size_kbytes = 1), small);
    assert_eq!(
        with(|spec| spec.coef_interpolation = CoefInterpolation::High),
        small
    );
    assert_eq!(small.len(), default.len());
    let worst = small
        .iter()
        .zip(&default)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f32::max);
    assert!(worst > 0.0 && worst < 5e-6, "{worst:e}");
}

/// The largest error against the sine as libsoxr's was measured: 40 000
/// inputs, and all but 2000 outputs at each end
fn sine_error(
    quality: Quality,
    frequency: f64,
    change: impl FnOnce(&mut RuntimeSpec),
) -> (Vec<f64>, f64) {
    let (from, to) = (44_100.0, 48_000.0);
    let sine = |rate: f64| move |index: usize| 0.5 * (TAU * frequency * index as f64 / rate).sin();
    let input: Vec<f64> = (0..40_000).map(sine(from)).collect();
    let mut runtime = RuntimeSpec::default();
    change(&mut runtime);
    let output = Resampler::<f64, f64>::builder(from, to, 1)
        .quality(quality)
        .runtime(runtime)
        .oneshot(&input[..])
        .unwrap();
    let expected = sine(to);
    let error = (2000..output.len() - 2000)
        .map(|index| (output[index] - expected(index)).abs())
        .fold(0.0, f64::max);
    (output, error)
}

/// Low, High and a table too small for the ratio's phases interpolate even a
/// ratio with a short fraction, as in libsoxr. Each errs at most
/// 1.25 times the larger of libsoxr's error for the same choice and the error
/// without interpolating, which leaves room for the rounding of `f32` at HQ.
#[test]
fn interpolating_stays_within_libsoxrs_error() {
    // Not 15 kHz, where the edge of the pass-band decides the error
    let frequencies = [0.05 * 44_100.0 / TAU, 100.0, 1000.0, 5000.0];
    let libsoxr = [
        (
            Quality::Low,
            [
                [7.05e-6, 6.87e-6, 6.87e-6],
                [7.24e-6, 7.06e-6, 7.06e-6],
                [5.16e-6, 4.98e-6, 4.98e-6],
                [3.60e-6, 3.45e-6, 3.45e-6],
            ],
        ),
        (
            Quality::Medium,
            [
                [2.98e-6, 3.01e-6, 3.01e-6],
                [3.11e-6, 3.11e-6, 3.11e-6],
                [2.72e-6, 2.72e-6, 2.72e-6],
                [3.10e-6, 3.07e-6, 3.07e-6],
            ],
        ),
        (
            Quality::High,
            [
                [2.92e-7, 3.04e-7, 3.04e-7],
                [3.47e-7, 3.48e-7, 3.48e-7],
                [3.81e-7, 4.17e-7, 4.17e-7],
                [8.08e-7, 8.08e-7, 8.08e-7],
            ],
        ),
        (
            Quality::VeryHigh,
            [[4.83e-8; 3], [1.38e-8; 3], [1.38e-7; 3], [6.89e-7; 3]],
        ),
    ];
    let changes: [fn(&mut RuntimeSpec); 3] = [
        |spec| spec.coef_interpolation = CoefInterpolation::Low,
        |spec| spec.coef_interpolation = CoefInterpolation::High,
        |spec| spec.coef_size_kbytes = 1,
    ];
    for (quality, errors) in libsoxr {
        for (frequency, errors) in frequencies.into_iter().zip(errors) {
            let (exact, exact_error) = sine_error(quality, frequency, |_| {});
            for (change, libsoxr) in changes.into_iter().zip(errors) {
                let (output, error) = sine_error(quality, frequency, change);
                assert_ne!(output, exact, "{quality:?} at {frequency} Hz");
                let bound = 1.25 * exact_error.max(libsoxr);
                assert!(
                    error <= bound,
                    "{quality:?} at {frequency} Hz: {error:e} over {bound:e}"
                );
            }
        }
    }
}
