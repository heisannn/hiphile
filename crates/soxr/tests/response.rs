//! Frequency response and the timing and gain of an impulse

mod common;

use std::f64::consts::TAU;

use common::spec;
use realfft::RealFftPlanner;
use soxr::{Phase, Quality, QualitySpec, Recipe, Resampler};

const IMPULSE_AT: usize = 20_000;

/// The output for a unit impulse at [`IMPULSE_AT`], and the position it
/// should peak at
fn impulse_response(quality: QualitySpec, rates: (f64, f64)) -> (Vec<f64>, f64) {
    let mut input = vec![0.0f64; 2 * IMPULSE_AT];
    input[IMPULSE_AT] = 1.0;
    let output = Resampler::<f64, f64>::builder(rates.0, rates.1, 1)
        .quality(quality)
        .oneshot(&input[..])
        .unwrap();
    (output, IMPULSE_AT as f64 * rates.1 / rates.0)
}

fn peak(response: &[f64]) -> usize {
    response
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .unwrap()
        .0
}

/// Of an upsampling by 2, in dB against a gain of 1, from a transform of
/// `len` samples. Bin `b` is at `4 * b / len` of the input's Nyquist
/// frequency.
fn response_db(quality: QualitySpec, len: usize) -> Vec<f64> {
    let (response, expected) = impulse_response(quality, (1.0, 2.0));
    let centre = expected as usize;
    let mut time = vec![0.0; len];
    let reach = 5000;
    time[..2 * reach].copy_from_slice(&response[centre - reach..centre + reach]);
    let fft = RealFftPlanner::<f64>::new().plan_fft_forward(len);
    let mut spectrum = fft.make_output_vec();
    fft.process(&mut time, &mut spectrum).unwrap();
    spectrum
        .iter()
        .map(|bin| 20.0 * (bin.norm() / 2.0).log10())
        .collect()
}

/// What was observed of libsoxr at 1 to 2: the largest deviation to 0.9 of
/// the pass-band in dB, and the -3 dB point
struct Observed {
    recipe: Recipe,
    deviation: f64,
    half_power: f64,
}

const fn observed(quality: Quality, steep: bool, deviation: f64, half_power: f64) -> Observed {
    Observed {
        recipe: Recipe::new(quality).steep(steep),
        deviation,
        half_power,
    }
}

const AT_ONE_TO_TWO: [Observed; 12] = [
    observed(Quality::Low, false, 4.4e-5, 0.812),
    observed(Quality::Medium, false, 2.3e-5, 0.951),
    observed(Quality::Bits16, false, 2.3e-5, 0.951),
    observed(Quality::High, false, 4.2e-6, 0.951),
    observed(Quality::Bits24, false, 1.1e-7, 0.951),
    observed(Quality::VeryHigh, false, 6.4e-9, 0.951),
    observed(Quality::Bits32, false, 3.6e-10, 0.950),
    observed(Quality::High, true, 3.6e-6, 0.991),
    observed(Quality::VeryHigh, true, 1.4e-9, 0.990),
    observed(Quality::LsrBest, false, 1.5e-7, 0.961),
    observed(Quality::LsrMedium, false, 6.6e-6, 0.903),
    observed(Quality::LsrFast, false, 9.3e-5, 0.803),
];

/// The stop-band within 3 dB of `(precision + 1) * 6.02` dB, the -3 dB point
/// within 0.003 of libsoxr's, and the pass-band within 10 times libsoxr's
/// deviation
#[test]
fn the_response_at_one_to_two_meets_libsoxrs_bounds() {
    let len = 1 << 20;
    let frequency = |bin: usize| 4.0 * bin as f64 / len as f64;
    for observed in AT_ONE_TO_TWO {
        let quality = QualitySpec::new(observed.recipe);
        let (passband_end, stopband_begin) = quality.band();
        let db = response_db(quality, len);
        let bins = 0..db.len();
        let deviation = bins
            .clone()
            .take_while(|bin| frequency(*bin) <= 0.9 * passband_end)
            .map(|bin| db[bin].abs())
            .fold(0.0, f64::max);
        let half_power = bins
            .clone()
            .find(|bin| frequency(*bin) > 0.5 && db[*bin] < -3.0)
            .map(frequency)
            .unwrap();
        let stopband = bins
            .filter(|bin| frequency(*bin) >= stopband_begin)
            .map(|bin| db[bin])
            .fold(f64::NEG_INFINITY, f64::max);
        let attenuation = (quality.precision + 1.0) * 6.0206;
        let name = format!("{:?}", observed.recipe);
        assert!(
            stopband <= -attenuation + 3.0,
            "{name}: stop-band {stopband:.1} dB"
        );
        assert!(
            (half_power - observed.half_power).abs() <= 0.003,
            "{name}: -3 dB at {half_power:.4}"
        );
        assert!(
            deviation <= 10.0 * observed.deviation,
            "{name}: pass-band {deviation:.2e} dB"
        );
    }
}

/// The gain of a sine at `fraction` of the lower Nyquist frequency, in dB,
/// by projection on the expected sine, and the level of the alias at
/// `alias` Hz if given
fn sine_gain(quality: QualitySpec, rates: (f64, f64), frequency: f64, alias: Option<f64>) -> f64 {
    let (input_rate, output_rate) = rates;
    let len = (input_rate * 0.5) as usize;
    let input: Vec<f64> = (0..len)
        .map(|index| (TAU * frequency * index as f64 / input_rate).sin())
        .collect();
    let output = Resampler::<f64, f64>::builder(input_rate, output_rate, 1)
        .quality(quality)
        .oneshot(&input[..])
        .unwrap();
    let measured = alias.unwrap_or(frequency);
    let margin = output.len() / 5;
    // Least squares on a sine and a cosine, exact for a sine at that frequency
    let [mut ss, mut sc, mut cc, mut ys, mut yc] = [0.0; 5];
    for (index, value) in output
        .iter()
        .enumerate()
        .take(output.len() - margin)
        .skip(margin)
    {
        let (sine, cosine) = (TAU * measured * index as f64 / output_rate).sin_cos();
        ss += sine * sine;
        sc += sine * cosine;
        cc += cosine * cosine;
        ys += value * sine;
        yc += value * cosine;
    }
    let determinant = ss * cc - sc * sc;
    let a = (ys * cc - yc * sc) / determinant;
    let b = (yc * ss - ys * sc) / determinant;
    20.0 * (a * a + b * b).sqrt().log10()
}

/// Off a ratio of whole numbers too, the -3 dB point lies within 0.003 of what
/// was observed of libsoxr at 1 to 2, and the pass-band stays flat
#[test]
fn the_response_off_whole_ratios_has_the_same_edges() {
    let cases = [
        (Recipe::new(Quality::High), 0.951, 4.2e-6),
        (Recipe::new(Quality::VeryHigh), 0.951, 6.4e-9),
        (Recipe::new(Quality::High).steep(true), 0.991, 3.6e-6),
    ];
    for rates in [(48_000.0f64, 44_100.0f64), (44_100.0, 48_000.0)] {
        let nyquist = rates.0.min(rates.1) / 2.0;
        for (recipe, half_power, deviation) in cases {
            let quality = QualitySpec::new(recipe);
            let gain = |fraction: f64| sine_gain(quality, rates, fraction * nyquist, None);
            assert!(gain(half_power - 0.003) > -3.0, "{recipe:?} {rates:?}");
            assert!(gain(half_power + 0.003) < -3.0, "{recipe:?} {rates:?}");
            let (passband_end, _) = quality.band();
            for fraction in [0.1, 0.5, 0.8, 0.9 * passband_end] {
                let error = gain(fraction).abs();
                assert!(
                    error <= 10.0 * deviation,
                    "{recipe:?} {rates:?} {fraction}: {error:.2e}"
                );
            }
        }
    }
}

#[test]
fn steep_widens_the_lsr_presets() {
    let rates = (44_100.0, 48_000.0);
    let frequency = 0.97 * 22_050.0;
    for quality in [Quality::LsrBest, Quality::LsrMedium, Quality::LsrFast] {
        let gain = |steep| {
            let quality = QualitySpec::new(Recipe::new(quality).steep(steep));
            sine_gain(quality, rates, frequency, None)
        };
        assert!(gain(true).abs() < 0.01, "{quality:?}");
        assert!(gain(false) < -3.0, "{quality:?}");
    }
}

#[test]
fn aliases_stay_in_the_stop_band() {
    let rates = (48_000.0, 44_100.0);
    for quality in [
        Quality::Low,
        Quality::Medium,
        Quality::High,
        Quality::VeryHigh,
    ] {
        let spec = spec(quality);
        let attenuation = (spec.precision + 1.0) * 6.0206;
        for frequency in [22_100.0, 22_500.0, 23_000.0, 23_500.0, 23_999.0] {
            let level = sine_gain(spec, rates, frequency, Some(44_100.0 - frequency));
            assert!(
                level <= -attenuation + 3.0,
                "{quality:?} {frequency}: {level:.1} dB"
            );
        }
    }
}

/// A linear-phase peak at the nearest output sample, and a gain of 1 at 0 Hz
#[test]
fn an_impulse_comes_out_on_time_with_unit_gain() {
    let cases = [
        (Quality::Quick, (1.0, 2.0)),
        (Quality::Quick, (44_100.0, 48_000.0)),
        (Quality::Low, (1.0, 2.0)),
        (Quality::Medium, (1.0, 2.0)),
        (Quality::High, (1.0, 2.0)),
        (Quality::High, (44_100.0, 48_000.0)),
        (Quality::High, (48_000.0, 44_100.0)),
        (Quality::High, (96_000.0, 44_100.0)),
        (Quality::VeryHigh, (1.0, 2.0)),
        (Quality::VeryHigh, (44_100.0, 96_000.0)),
        (Quality::VeryHigh, (2.0, 1.0)),
        (Quality::Bits32, (44_100.0, 50_000.123)),
        (Quality::High, (192_000.0, 44_100.0)),
        (Quality::VeryHigh, (768_000.0, 44_100.0)),
        (Quality::High, (44_100.0, 768_000.0)),
        (Quality::High, (100_000.0, 44_100.0 / std::f64::consts::PI)),
    ];
    for (quality, rates) in cases {
        let (response, expected) = impulse_response(spec(quality), rates);
        let peak = peak(&response) as f64;
        assert!(
            (peak - expected).abs() <= 0.5,
            "{quality:?} {rates:?}: {peak} for {expected}"
        );
        let gain = response.iter().sum::<f64>() * rates.0 / rates.1;
        // An unfiltered cubic's samples do not sum to 1 off whole ratios, as
        // libsoxr's did not (0.99996), though it passes 0 Hz exactly
        let tolerance = if quality == Quality::Quick {
            1e-4
        } else {
            1e-5
        };
        assert!(
            (gain - 1.0).abs() <= tolerance,
            "{quality:?} {rates:?}: {gain}"
        );
    }
}

#[test]
fn quick_halving_takes_every_other_sample() {
    let (response, expected) = impulse_response(spec(Quality::Quick), (2.0, 1.0));
    assert_eq!(peak(&response) as f64, expected);
    assert_eq!(response.iter().sum::<f64>(), 1.0);
}

fn with_phase(quality: Quality, steep: bool, phase_response: f64) -> QualitySpec {
    let mut spec = QualitySpec::new(Recipe::new(quality).steep(steep));
    spec.phase_response = phase_response;
    spec
}

/// The first and the last output over 1e-7, from the peak, as libsoxr's were
/// measured
fn reach(response: &[f64]) -> (isize, isize) {
    let peak = peak(response) as isize;
    let first = response.iter().position(|y| y.abs() > 1e-7).unwrap() as isize;
    let last = response.iter().rposition(|y| y.abs() > 1e-7).unwrap() as isize;
    (first - peak, last - peak)
}

/// At any phase the main peak lies within 1.5 output samples of the input's
/// time, and the gain at 0 Hz is 1
#[test]
fn every_phase_peaks_on_time_with_unit_gain() {
    let rates = [
        (1.0, 2.0),
        (2.0, 1.0),
        (44_100.0, 48_000.0),
        (48_000.0, 44_100.0),
        (44_100.0, 96_000.0),
        (192_000.0, 44_100.0),
    ];
    for (quality, steep) in [
        (Quality::Low, false),
        (Quality::High, false),
        (Quality::High, true),
        (Quality::VeryHigh, false),
    ] {
        for phase_response in [0.0, 10.0, 25.0, 75.0, 100.0] {
            for rates in rates {
                let spec = with_phase(quality, steep, phase_response);
                let (response, expected) = impulse_response(spec, rates);
                let peak = peak(&response) as f64;
                let name = format!("{quality:?} {steep} {phase_response} {rates:?}");
                assert!(
                    (peak - expected).abs() <= 1.5,
                    "{name}: {peak} for {expected}"
                );
                let gain = response.iter().sum::<f64>() * rates.0 / rates.1;
                assert!((gain - 1.0).abs() <= 1e-5, "{name}: {gain}");
            }
        }
    }
}

/// At HQ from 1 to 2, minimum phase answers a few samples before its peak,
/// maximum phase a few after, and the phases over 50 mirror those under
#[test]
fn minimum_phase_answers_early_and_maximum_phase_mirrors_it() {
    let quality = Quality::High;
    let response =
        |phase_response| impulse_response(with_phase(quality, false, phase_response), (1.0, 2.0));
    let (minimum, centre) = response(0.0);
    let (before, after) = reach(&minimum);
    assert!((-12..0).contains(&before), "{before}");
    assert!(after > 300, "{after}");
    let (linear, _) = response(50.0);
    for (early, late) in [(0.0, 100.0), (25.0, 75.0)] {
        let (early, _) = response(early);
        let (late, _) = response(late);
        let centre = centre as usize;
        let largest = (0..2000)
            .map(|offset| (early[centre + offset] - late[centre - offset]).abs())
            .fold(0.0, f64::max);
        // HQ computes in `f32`
        assert!(largest < 1e-6, "{largest:e}");
        let (early_before, _) = reach(&early);
        let (linear_before, _) = reach(&linear);
        assert!(early_before >= linear_before - 100, "{early_before}");
    }
}

/// Away from linear phase the response meets the same bounds as at linear
/// phase
#[test]
fn every_phase_keeps_the_response() {
    let len = 1 << 20;
    let frequency = |bin: usize| 4.0 * bin as f64 / len as f64;
    for observed in AT_ONE_TO_TWO
        .iter()
        .filter(|observed| observed.recipe.phase == Phase::Linear)
        .take(9)
    {
        for phase_response in [0.0, 25.0, 100.0] {
            let mut quality = QualitySpec::new(observed.recipe);
            quality.phase_response = phase_response;
            let (passband_end, stopband_begin) = quality.band();
            let db = response_db(quality, len);
            let deviation = (0..db.len())
                .take_while(|bin| frequency(*bin) <= 0.9 * passband_end)
                .map(|bin| db[bin].abs())
                .fold(0.0, f64::max);
            let half_power = (0..db.len())
                .find(|bin| frequency(*bin) > 0.5 && db[*bin] < -3.0)
                .map(frequency)
                .unwrap();
            let stopband = (0..db.len())
                .filter(|bin| frequency(*bin) >= stopband_begin)
                .map(|bin| db[bin])
                .fold(f64::NEG_INFINITY, f64::max);
            let attenuation = (quality.precision + 1.0) * 6.0206;
            let name = format!("{:?} {phase_response}", observed.recipe);
            assert!(
                stopband <= -attenuation + 3.0,
                "{name}: stop-band {stopband:.1} dB"
            );
            assert!(
                (half_power - observed.half_power).abs() <= 0.003,
                "{name}: -3 dB at {half_power:.4}"
            );
            assert!(
                deviation <= 10.0 * observed.deviation,
                "{name}: pass-band {deviation:.2e} dB"
            );
        }
    }
}

#[test]
fn phase_bits_are_their_responses() {
    let input = common::noise(20_000, 3);
    for (phase, response) in [
        (Phase::Intermediate, 25.0),
        (Phase::Minimum, 0.0),
        (Phase::Maximum, 100.0),
    ] {
        let by_bits: Vec<f32> = common::oneshot(
            (44_100.0, 48_000.0),
            Recipe::new(Quality::High).with_phase(phase),
            1,
            &input,
        );
        let by_response: Vec<f32> = common::oneshot(
            (44_100.0, 48_000.0),
            with_phase(Quality::High, false, response),
            1,
            &input,
        );
        assert_eq!(by_bits, by_response, "{phase:?}");
    }
}

/// The basis of four-point cubic Lagrange interpolation for the
/// sample `offset` input samples from the time
fn lagrange(offset: f64) -> f64 {
    let x = offset.abs();
    if x < 1.0 {
        (x + 1.0) * (x - 1.0) * (x - 2.0) / 2.0
    } else if x < 2.0 {
        let x = x - 1.0;
        -x * (x - 1.0) * (x - 2.0) / 6.0
    } else {
        0.0
    }
}

#[test]
fn quick_answers_with_the_lagrange_basis() {
    for factor in [2.0, 3.0, 4.0] {
        let (response, expected) = impulse_response(spec(Quality::Quick), (1.0, factor));
        for (index, value) in response.iter().enumerate() {
            let offset = (index as f64 - expected) / factor;
            assert!(
                (value - lagrange(offset)).abs() <= 1e-6,
                "1 to {factor}, {index}: {value} for {}",
                lagrange(offset)
            );
        }
        let centre = expected as usize;
        let at = |offset: usize| response[centre + offset];
        match factor as usize {
            2 => assert_eq!([at(1), at(2), at(3)], [0.5625, 0.0, -0.0625]),
            4 => assert_eq!(
                [at(1), at(2), at(3), at(4), at(5), at(6), at(7), at(8)],
                [
                    0.8203125, 0.5625, 0.2734375, 0.0, -0.0546875, -0.0625, -0.0390625, 0.0
                ]
            ),
            _ => {}
        }
    }
}
