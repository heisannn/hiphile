use std::f64::consts::TAU;
use std::thread;

use r8brain::{CacheLimits, Error, Options, Phase, Resampler};

/// One of each route through the library
const RATES: [(f64, f64); 23] = [
    (48_000.0, 24_000.0),
    (48_000.0, 16_000.0),
    (48_000.0, 32_000.0),
    (32_000.0, 48_000.0),
    (48_000.0, 36_000.0),
    (44_100.0, 88_200.0),
    (44_100.0, 705_600.0),
    (44_100.0, 132_300.0),
    (44_100.0, 1_058_400.0),
    (44_100.0, 48_000.0),
    (44_100.0, 96_000.0),
    (44_100.0, 768_000.0),
    (8_000.0, 44_100.0),
    (44_100.0, 47_999.5),
    (44_100.0, 192_003.0),
    (48_000.0, 44_100.0),
    (96_000.0, 44_100.0),
    (768_000.0, 44_100.0),
    (384_000.0, 48_000.0),
    (576_000.0, 48_000.0),
    (768_000.0, 44_101.0),
    (192_000.0, 61_000.0),
    (1_000_000.0, 44_100.0),
];

fn sine(frequency: f64, rate: f64, len: usize) -> Vec<f64> {
    (0..len)
        .map(|index| (TAU * frequency * index as f64 / rate).sin())
        .collect()
}

fn resampler(source: f64, target: f64) -> Resampler {
    Resampler::new(source, target, Options::BITS_24).unwrap()
}

/// As much as `input` brings, passed on in the given lengths, over and over
fn in_chunks(resampler: &mut Resampler, input: &[f64], lengths: &[usize]) -> Vec<f64> {
    let mut output = Vec::new();
    let mut rest = input;
    for length in lengths.iter().cycle() {
        if rest.is_empty() {
            break;
        }
        let (chunk, others) = rest.split_at(rest.len().min(*length));
        output.extend_from_slice(resampler.process(chunk));
        rest = others;
    }
    output
}

/// Well inside what `input_before_output` says a fifth of a second of output
/// takes, and past the ramp at the start
fn steady_part(output: &[f64], target: f64) -> &[f64] {
    let margin = (target / 10.0) as usize;
    &output[margin..output.len() - margin.min(output.len() / 4)]
}

#[test]
fn equal_rates_give_the_input_back() {
    let mut resampler = resampler(44_100.0, 44_100.0);
    let input = [0.25, -0.5, 0.75];
    assert_eq!(resampler.process(&input), input);
    assert_eq!(resampler.input_before_output(7), 7);
}

/// The time and level of a sine in the pass-band stay exactly what they were,
/// and whatever else the steps add stays near the requested attenuation.
#[test]
fn a_sine_in_the_pass_band_comes_out_on_time_and_clean() {
    for (source, target) in RATES {
        let frequency = 0.2 * source.min(target);
        let seconds = 0.6;
        let input = sine(frequency, source, (source * seconds) as usize);
        let mut resampler = resampler(source, target);
        let output = resampler.process(&input);
        assert!(!output.is_empty(), "{source} -> {target}");

        let start = (target * 0.1) as usize;
        let mut largest_error = 0.0f64;
        for (index, sample) in output.iter().enumerate().skip(start) {
            let expected = (TAU * frequency * index as f64 / target).sin();
            largest_error = largest_error.max((sample - expected).abs());
        }
        assert!(
            largest_error < 1e-7,
            "{source} -> {target}: off by {largest_error}"
        );
    }
}

/// Above the target's Nyquist frequency, nothing is to come out at all, on any
/// route that goes down.
#[test]
fn a_sine_above_the_target_band_is_gone() {
    let mut minimum = Options::BITS_24;
    minimum.phase = Phase::Minimum;
    for options in [Options::BITS_24, minimum] {
        for (source, target) in RATES.into_iter().filter(|(source, target)| target < source) {
            let frequency = target * 0.5 + (source - target) * 0.25;
            let input = sine(frequency, source, source as usize);
            let mut resampler = Resampler::new(source, target, options).unwrap();
            let output = resampler.process(&input);
            let steady = steady_part(output, target);
            let peak = steady
                .iter()
                .fold(0.0f64, |peak, sample| peak.max(sample.abs()));
            // 180 dB is 1e-9, with some room left for the sum of the steps
            assert!(
                peak < 3e-9,
                "{source} -> {target}, {:?}: {frequency} Hz at {peak}",
                options.phase
            );
        }
    }
}

#[test]
fn how_the_input_is_cut_up_does_not_matter() {
    let input = sine(997.0, 48_000.0, 40_000);
    for (source, target) in RATES {
        let whole = resampler(source, target).process(&input).to_vec();
        for lengths in [&[1][..], &[7, 64, 1_000, 3, 5_000, 1, 1, 333]] {
            let cut_up = in_chunks(&mut resampler(source, target), &input, lengths);
            assert_eq!(whole.len(), cut_up.len(), "{source} -> {target}");
            for (a, b) in whole.iter().zip(&cut_up) {
                // Interpolation restarts its position at the end of a call
                assert!((a - b).abs() < 1e-11, "{source} -> {target}");
            }
        }
    }
}

#[test]
fn clearing_starts_over() {
    let input = sine(997.0, 48_000.0, 20_000);
    for (source, target) in RATES {
        let mut resampler = resampler(source, target);
        let first = resampler.process(&input).to_vec();
        resampler.clear();
        assert_eq!(first, resampler.process(&input), "{source} -> {target}");
    }
}

/// `Resampler::new` shares its filters, so later ones have nothing of their own
/// to work out, and the state of one does not reach another.
#[test]
fn resamplers_do_not_disturb_each_other() {
    let input = sine(997.0, 48_000.0, 20_000);
    let alone = resampler(44_100.0, 48_000.0).process(&input).to_vec();
    let mut first = resampler(44_100.0, 48_000.0);
    let mut second = resampler(44_100.0, 48_000.0);
    second.process(&sine(5_000.0, 48_000.0, 1_234));
    assert_eq!(first.process(&input), alone);
}

#[test]
fn resamplers_of_several_threads_come_out_the_same() {
    let input = sine(997.0, 48_000.0, 20_000);
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let input = input.clone();
            thread::spawn(move || resampler(37_000.0, 53_000.0).process(&input).to_vec())
        })
        .collect();
    let mut results = threads.into_iter().map(|thread| thread.join().unwrap());
    let first = results.next().unwrap();
    assert!(results.all(|result| result == first));
}

#[test]
fn the_input_before_the_output_is_as_stated() {
    for (source, target) in RATES {
        let mut resampler = resampler(source, target);
        let stated = resampler.input_before_output(0);
        let mut fed = 0;
        while resampler.process(&[0.0]).is_empty() {
            fed += 1;
        }
        assert_eq!(fed, stated, "{source} -> {target}");
    }
}

#[test]
fn the_input_required_brings_that_much_output() {
    for (source, target) in RATES {
        let mut resampler = resampler(source, target);
        assert_eq!(resampler.input_required_for_output(0), 0);
        let wanted = 1_000;
        let required = resampler.input_required_for_output(wanted);
        let produced = resampler.process(&vec![0.0; required]).len();
        assert!(produced >= wanted, "{source} -> {target}: {produced}");
    }
}

#[test]
fn oneshot_fills_the_output_and_starts_over() {
    let input = sine(997.0, 48_000.0, 10_000);
    let mut resampler = resampler(48_000.0, 96_000.0);
    let mut twice = [vec![0.0; 25_000], vec![0.0; 25_000]];
    for output in &mut twice {
        resampler.oneshot(&input, output);
    }
    assert_eq!(twice[0], twice[1]);

    // Past the input and its filter's tail, silence
    let tail = &twice[0][24_000..];
    assert!(tail.iter().all(|sample| sample.abs() < 1e-8));
    let expected = (TAU * 997.0 * 5_000.0 / 96_000.0).sin();
    assert!((twice[0][5_000] - expected).abs() < 1e-7);
}

#[test]
fn minimum_phase_has_less_latency_at_the_same_level() {
    for (source, target) in RATES {
        let mut minimum = Options::BITS_24;
        minimum.phase = Phase::Minimum;
        let mut resampler_of_minimum = Resampler::new(source, target, minimum).unwrap();
        assert!(
            (0.0..1.0).contains(&resampler_of_minimum.latency_frac()),
            "{source} -> {target}"
        );
        assert!(
            resampler_of_minimum.input_before_output(0)
                < resampler(source, target).input_before_output(0),
            "{source} -> {target}"
        );

        let frequency = 0.05 * source.min(target);
        let output = resampler_of_minimum
            .process(&sine(frequency, source, (source * 0.6) as usize))
            .to_vec();
        let steady = steady_part(&output, target);
        let peak = steady
            .iter()
            .fold(0.0f64, |peak, sample| peak.max(sample.abs()));
        assert!((peak - 1.0).abs() < 1e-2, "{source} -> {target}: {peak}");
    }
}

#[test]
fn a_number_that_is_none_passes_and_does_not_stay() {
    for (source, target) in RATES {
        let mut resampler = resampler(source, target);
        let mut input = sine(997.0, 48_000.0, 4_000);
        input[2_000] = f64::NAN;
        input[2_001] = f64::INFINITY;
        resampler.process(&input);

        resampler.clear();
        let clean = sine(997.0, 48_000.0, 200_000);
        let output = resampler.process(&clean);
        assert!(
            output.iter().all(|sample| sample.is_finite()),
            "{source} -> {target}"
        );
    }
}

#[test]
fn values_outside_the_ranges_are_refused() {
    let options = Options::default();
    for rate in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert_eq!(
            Resampler::new(rate, 48_000.0, options).err(),
            Some(Error::InvalidSampleRate)
        );
        assert_eq!(
            Resampler::new(48_000.0, rate, options).err(),
            Some(Error::InvalidSampleRate)
        );
    }
    for transition_band in [0.49, 45.1, f64::NAN] {
        let mut options = options;
        options.transition_band = transition_band;
        assert_eq!(
            Resampler::new(44_100.0, 48_000.0, options).err(),
            Some(Error::InvalidTransitionBand)
        );
    }
    for attenuation in [48.9, 218.1, f64::NAN] {
        let mut options = options;
        options.attenuation = attenuation;
        assert_eq!(
            Resampler::new(44_100.0, 48_000.0, options).err(),
            Some(Error::InvalidAttenuation)
        );
    }
}

#[test]
fn other_values_outside_the_ranges_are_refused() {
    let mut zero_length = Options::default();
    zero_length.max_input_len = 0;
    assert_eq!(
        Resampler::new(44_100.0, 48_000.0, zero_length).err(),
        Some(Error::InvalidMaxInputLen)
    );
    let mut no_fractions = Options::default();
    no_fractions.filter_fractions = Some(4);
    assert_eq!(
        Resampler::new(44_100.0, 48_000.0, no_fractions).err(),
        Some(Error::InvalidFilterFractions)
    );
    let mut few_filters = CacheLimits::default();
    few_filters.filters = 1;
    let mut few_banks = CacheLimits::default();
    few_banks.filter_banks = 1;
    for limits in [few_filters, few_banks] {
        assert_eq!(limits.apply(), Err(Error::InvalidCacheLimit));
    }
}

#[test]
fn the_most_output_is_never_passed() {
    for (source, target) in RATES {
        for max_input_len in [1, 64, 1000] {
            let mut options = Options::BITS_24;
            options.max_input_len = max_input_len;
            let mut resampler = Resampler::new(source, target, options).unwrap();
            let most = resampler.max_output_len();
            let input = sine(997.0_f64.min(target / 8.0), source, 40_000);
            for (index, chunk) in input.chunks(max_input_len).enumerate() {
                // Also lengths that do not line up with what steps take
                let chunk = &chunk[..chunk.len().min(1 + index % max_input_len)];
                let output = resampler.process(chunk);
                assert!(
                    output.len() <= most,
                    "{source} -> {target}, {max_input_len}: {} of {most}",
                    output.len()
                );
            }
        }
    }
}

#[test]
fn the_planned_input_length_changes_nothing_in_the_output() {
    let input = sine(997.0, 44_100.0, 50_000);
    let output_of = |max_input_len| {
        let mut options = Options::BITS_24;
        options.max_input_len = max_input_len;
        let mut resampler = Resampler::new(44_100.0, 48_000.0, options).unwrap();
        // More than planned in one call, which makes the buffers grow
        resampler.process(&input).to_vec()
    };
    assert_eq!(output_of(1), output_of(1 << 16));
}

#[test]
fn oneshot_passes_on_as_much_as_was_planned() {
    let input = sine(997.0, 44_100.0, 2_000);
    let output_of = |max_input_len| {
        let mut options = Options::BITS_24;
        options.max_input_len = max_input_len;
        let mut resampler = Resampler::new(44_100.0, 48_000.0, options).unwrap();
        let mut output = vec![0.0; 2_000];
        resampler.oneshot(&input, &mut output);
        output
    };
    assert_eq!(output_of(16), output_of(4096));
}

#[test]
fn the_extended_fft_takes_more_latency_for_the_same_output() {
    let input = sine(997.0, 44_100.0, 60_000);
    for (source, target) in RATES {
        let mut extended = Options::BITS_24;
        extended.extended_fft = true;
        let mut plain = resampler(source, target);
        let mut longer = Resampler::new(source, target, extended).unwrap();
        assert!(
            longer.input_before_output(0) >= plain.input_before_output(0),
            "{source} -> {target}"
        );

        let wanted = (target as usize / 40).min(4_000);
        let required = plain
            .input_required_for_output(wanted)
            .max(longer.input_required_for_output(wanted))
            .min(input.len());
        let plain_output = plain.process(&input[..required]).to_vec();
        let longer_output = longer.process(&input[..required]);
        let shared = plain_output.len().min(longer_output.len());
        for (a, b) in plain_output[..shared].iter().zip(&longer_output[..shared]) {
            assert!((a - b).abs() < 1e-12, "{source} -> {target}");
        }
    }
}

#[test]
fn rates_that_step_wholly_can_interpolate_instead() {
    let mut options = Options::BITS_24;
    options.whole_stepping = false;
    let mut resampler = Resampler::new(44_100.0, 48_000.0, options).unwrap();
    let output = resampler.process(&sine(997.0, 44_100.0, 60_000)).to_vec();
    for (index, sample) in steady_part(&output, 48_000.0).iter().enumerate() {
        let time = (index + 4_800) as f64 / 48_000.0;
        let expected = (TAU * 997.0 * time).sin();
        assert!((sample - expected).abs() < 1e-7, "{index}");
    }
}

#[test]
fn fewer_filter_fractions_interpolate_worse() {
    let worst_error_of = |filter_fractions| {
        let mut options = Options::BITS_24;
        options.filter_fractions = filter_fractions;
        let mut resampler = Resampler::new(44_100.0, 47_999.5, options).unwrap();
        let output = resampler.process(&sine(997.0, 44_100.0, 60_000)).to_vec();
        steady_part(&output, 47_999.5)
            .iter()
            .enumerate()
            .map(|(index, sample)| {
                let time = (index + 4_799) as f64 / 47_999.5;
                (sample - (TAU * 997.0 * time).sin()).abs()
            })
            .fold(0.0, f64::max)
    };
    let automatic = worst_error_of(None);
    assert!(automatic < 1e-7, "{automatic}");
    let few = worst_error_of(Some(5));
    assert!(few > automatic * 10.0, "{few} against {automatic}");
    assert!(worst_error_of(Some(20_000)) < 1e-7);
}

#[test]
fn the_fewest_filter_fractions_stay_finite() {
    let mut options = Options::BITS_24;
    options.filter_fractions = Some(5);
    options.whole_stepping = false;
    for (source, target) in RATES {
        let mut resampler = Resampler::new(source, target, options).unwrap();
        let input = sine(997.0_f64.min(target / 8.0), source, 40_000);
        let output = resampler.process(&input);
        assert!(
            output.iter().all(|sample| sample.is_finite()),
            "{source} -> {target}"
        );
    }
}

#[test]
fn smaller_caches_leave_the_output_alone() {
    let before = resampler(44_100.0, 48_000.0)
        .process(&sine(997.0, 44_100.0, 40_000))
        .to_vec();
    let mut limits = CacheLimits::default();
    limits.filters = 2;
    limits.filter_banks = 2;
    limits.apply().unwrap();
    let after = resampler(44_100.0, 48_000.0)
        .process(&sine(997.0, 44_100.0, 40_000))
        .to_vec();
    CacheLimits::default().apply().unwrap();
    assert_eq!(before, after);
}

/// The original's limits, and the rates a device could ask of them
#[test]
fn every_end_of_the_ranges_works() {
    for (source, target) in [
        (1.0, 2.0),
        (8_000.0, 768_000.0),
        (768_000.0, 8_000.0),
        (44_100.0, 1_000_000_000.0),
    ] {
        for transition_band in [0.5, 45.0] {
            for attenuation in [49.0, 218.0] {
                let mut options = Options::default();
                options.transition_band = transition_band;
                options.attenuation = attenuation;
                let mut resampler = Resampler::new(source, target, options).unwrap();
                let required = resampler.input_required_for_output(16);
                assert!(resampler.process(&vec![0.0; required]).len() >= 16);
            }
        }
    }
}
