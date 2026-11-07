//! What r8brain-free-src itself makes of the same input. See
//! `upstream_reference/generate.cpp`.

use r8brain::{Options, Phase, Resampler};

const EXPECTED: &str = include_str!("upstream_reference/expected.txt");
const EXPECTED_EXTENDED_FFT: &str = include_str!("upstream_reference/expected_extended_fft.txt");
const EXPECTED_FILTER_TEST: &str = include_str!("upstream_reference/expected_filter_test.txt");
const INPUT_LEN: usize = 60_000;
/// Rounding of two different FFTs
const LINEAR_PHASE_TOLERANCE: f64 = 1e-13;
/// The minimum-phase transform takes logarithms of the FFT's rounding noise,
/// where upstream's own FFT back-ends differ by this order too
const MINIMUM_PHASE_TOLERANCE: f64 = 1e-3;

fn input() -> Vec<f64> {
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    (0..INPUT_LEN)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 * (1.0 / 9_007_199_254_740_992.0) - 1.0
        })
        .collect()
}

/// Each line of `expected` against the options that `base` leaves
fn assert_routes_come_out_as_upstream_does(expected: &str, routes: usize, base: Options) {
    let input = input();
    assert_eq!(expected.lines().count(), routes);
    for line in expected.lines() {
        let fields: Vec<f64> = line
            .split(' ')
            .map(|field| field.parse().unwrap())
            .collect();
        let &[
            source,
            target,
            transition_band,
            attenuation,
            minimum,
            filter_fractions,
            before,
            count,
            frac,
            ref samples @ ..,
        ] = fields.as_slice()
        else {
            panic!("{line}");
        };
        let is_minimum = minimum == 1.0;
        let route = format!(
            "{source} -> {target}, {transition_band}%, {attenuation} dB, minimum {is_minimum}, \
             {filter_fractions} fractions"
        );

        let mut options = base;
        options.transition_band = transition_band;
        options.attenuation = attenuation;
        options.phase = if is_minimum {
            Phase::Minimum
        } else {
            Phase::Linear
        };
        options.filter_fractions = (filter_fractions > 0.0).then_some(filter_fractions as u32);
        let mut resampler = Resampler::new(source, target, options).unwrap();
        assert_eq!(resampler.input_before_output(0) as f64, before, "{route}");
        let tolerance = if is_minimum {
            MINIMUM_PHASE_TOLERANCE
        } else {
            LINEAR_PHASE_TOLERANCE
        };
        assert!(
            (resampler.latency_frac() - frac).abs() <= tolerance,
            "{route}"
        );

        let output = resampler.process(&input);
        assert_eq!(output.len() as f64, count, "{route}");
        for (position, expected) in samples.iter().enumerate() {
            let index = position * (output.len() - 1) / (samples.len() - 1);
            let difference = (output[index] - expected).abs();
            assert!(
                difference <= tolerance,
                "{route}: sample {index} off by {difference}"
            );
        }
    }
}

#[test]
fn every_route_comes_out_as_upstream_does() {
    assert_routes_come_out_as_upstream_does(EXPECTED, 31, Options::default());
}

#[test]
fn every_route_with_the_extended_fft_comes_out_as_upstream_does() {
    let mut options = Options::default();
    options.extended_fft = true;
    assert_routes_come_out_as_upstream_does(EXPECTED_EXTENDED_FFT, 31, options);
}

#[test]
fn interpolation_of_any_number_of_fractions_comes_out_as_upstream_does() {
    let mut options = Options::default();
    options.whole_stepping = false;
    assert_routes_come_out_as_upstream_does(EXPECTED_FILTER_TEST, 9, options);
}
