use realfft::RealFftPlanner;

use super::*;
use crate::design::{self, Kernel};
use crate::{Quality, QualitySpec, Recipe};

/// The steep filter of every preset with a filter, as the chain designs it,
/// with its precision and attenuation
fn filters() -> Vec<(String, Vec<f64>, f64)> {
    let qualities = [
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
    qualities
        .iter()
        .flat_map(|quality| [false, true].map(|steep| Recipe::new(*quality).steep(steep)))
        .map(|recipe| {
            let spec = QualitySpec::new(recipe);
            let (passband_end, stopband_begin) = spec.band();
            let attenuation = design::attenuation(spec.precision);
            let kernel =
                Kernel::low_pass(passband_end / 4.0, stopband_begin / 4.0, attenuation, 1.0);
            let taps = kernel.with_half_len(kernel.half_len().ceil()).taps();
            (format!("{recipe:?}"), taps, attenuation)
        })
        .collect()
}

fn linear(precision: f64, passband_end: f64) -> (Vec<f64>, f64) {
    let attenuation = design::attenuation(precision);
    let kernel = Kernel::low_pass(passband_end / 4.0, 0.25, attenuation, 1.0);
    let taps = kernel.with_half_len(kernel.half_len().ceil()).taps();
    (taps, attenuation)
}

fn magnitude(taps: &[f64], len: usize) -> Vec<f64> {
    let fft = RealFftPlanner::<f64>::new().plan_fft_forward(len);
    let mut time = vec![0.0; len];
    time[..taps.len()].copy_from_slice(taps);
    let mut spectrum = fft.make_output_vec();
    fft.process(&mut time, &mut spectrum).unwrap();
    spectrum.iter().map(|bin| bin.norm()).collect()
}

#[test]
fn linear_phase_is_left_alone() {
    let (taps, attenuation) = linear(20.0, 0.9137);
    assert_eq!(
        shaped(&taps, LINEAR, attenuation),
        (taps.clone(), taps.len() / 2)
    );
}

#[test]
fn the_magnitude_is_kept() {
    for (name, taps, attenuation) in filters() {
        for response in [0.0, 25.0] {
            let (shaped, _) = shaped(&taps, response, attenuation);
            let len = 8 * shaped.len().max(taps.len()).next_power_of_two();
            let largest = magnitude(&taps, len)
                .iter()
                .zip(magnitude(&shaped, len))
                .map(|(before, after)| (before - after).abs())
                .fold(0.0, f64::max);
            let allowed = 10f64.powf(-(attenuation + CUT_DB) / 20.0) + 1e-14;
            assert!(
                largest <= allowed,
                "{name} {response}: {largest:e} over {allowed:e}"
            );
            let sum: f64 = shaped.iter().sum();
            assert!((sum - 1.0).abs() < 1e-14, "{sum}");
        }
    }
}

/// The minimum-phase filter of an FIR filter is no longer than it, and peaks
/// within a few taps of its start
#[test]
fn minimum_phase_is_short_and_early() {
    for (name, taps, attenuation) in filters() {
        let (shaped, peak) = shaped(&taps, 0.0, attenuation);
        assert!(
            shaped.len() <= taps.len(),
            "{name}: {} taps for {}",
            shaped.len(),
            taps.len()
        );
        assert!(peak <= 12, "{name}: peak at {peak}");
    }
}

#[test]
fn maximum_phase_mirrors_minimum_phase() {
    let (taps, attenuation) = linear(24.0, 0.9123);
    for response in [0.0, 10.0, 37.5] {
        let (early, early_peak) = shaped(&taps, response, attenuation);
        let (late, late_peak) = shaped(&taps, 100.0 - response, attenuation);
        let mirrored: Vec<f64> = late.iter().rev().copied().collect();
        assert_eq!(mirrored, early, "{response}");
        assert_eq!(late_peak, late.len() - 1 - early_peak);
    }
}

/// Between minimum and linear phase, the response reaches further before the
/// peak, as it did in libsoxr
#[test]
fn intermediate_phase_lies_between() {
    let (taps, attenuation) = linear(20.0, 0.9137);
    let before: Vec<usize> = [0.0, 10.0, 25.0, 40.0, LINEAR]
        .iter()
        .map(|response| {
            let (shaped, peak) = shaped(&taps, *response, attenuation);
            let first = shaped.iter().position(|tap| tap.abs() > 1e-6).unwrap();
            peak - first
        })
        .collect();
    assert!(before.is_sorted(), "{before:?}");
    assert!(before[0] <= 8, "{before:?}");
}
