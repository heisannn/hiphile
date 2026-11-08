//! The streaming contract: lengths, the delay, splitting into calls, the end of
//! the input and clearing

mod common;

use common::{QUALITIES, RATES, Random, noise, oneshot, spec, streamed, total_output};
use soxr::{Dither, Error, IoSpec, Quality, QualitySpec, Recipe, Resampler, Sample};

/// Any split into calls gives the output of one call, to the bit
fn check_split_invariance(len: usize) {
    for quality in QUALITIES {
        for rates in RATES {
            let input = noise(len, 7);
            let whole: Vec<f32> = oneshot(rates, spec(quality), 1, &input);
            let mut resampler = Resampler::<f32, f32>::builder(rates.0, rates.1, 1)
                .quality(spec(quality))
                .build()
                .unwrap();
            let split = streamed(&mut resampler, &input, 11, 4097, 3000);
            assert_eq!(
                split.len(),
                total_output(len, rates),
                "{quality:?} {rates:?}"
            );
            assert!(
                split
                    .iter()
                    .zip(&whole)
                    .all(|(a, b)| a.to_bits() == b.to_bits()),
                "{quality:?} {rates:?}"
            );
            assert_eq!(split.len(), whole.len());
        }
    }
}

#[test]
fn any_split_gives_the_same_bits() {
    check_split_invariance(200_000);
}

/// With the steep filter too
#[test]
fn any_split_gives_the_same_bits_at_any_phase() {
    let input = noise(60_000, 5);
    for (recipe, phase_response) in [
        (Recipe::new(Quality::High), 0.0),
        (Recipe::new(Quality::High), 37.5),
        (Recipe::new(Quality::VeryHigh), 100.0),
        (Recipe::new(Quality::High).steep(true), 25.0),
    ] {
        let mut quality = QualitySpec::new(recipe);
        quality.phase_response = phase_response;
        for rates in RATES {
            let whole: Vec<f32> = oneshot(rates, quality, 1, &input);
            let mut resampler = Resampler::<f32, f32>::builder(rates.0, rates.1, 1)
                .quality(quality)
                .build()
                .unwrap();
            let split = streamed(&mut resampler, &input, 13, 4097, 3000);
            assert_eq!(split, whole, "{recipe:?} {phase_response} {rates:?}");
            let delay = resampler.delay();
            assert_eq!(delay, 0.0, "{recipe:?} {phase_response} {rates:?}");
        }
    }
}

#[test]
fn the_output_is_as_long_as_libsoxr_computes() {
    let rates = [
        (44_100.0, 48_000.0),
        (48_000.0, 44_100.0),
        (44_100.0, 96_000.0),
        (96_000.0, 44_100.0),
        (44_100.0, 88_200.0),
        (88_200.0, 44_100.0),
        (1.0, 2.0),
        (2.0, 1.0),
        (1.0, 3.0),
        (3.0, 1.0),
        (44_100.0, 192_000.0),
        (192_000.0, 44_100.0),
        (8_000.0, 44_100.0),
        (44_100.0, 8_000.0),
        (48_000.0, 32_000.0),
        (44_100.0, 50_000.123),
        (768_000.0, 44_100.0),
    ];
    let lengths = [0, 1, 2, 3, 7, 10, 100, 1001, 44_100, 12_345];
    let qualities = [
        Quality::Quick,
        Quality::Low,
        Quality::Medium,
        Quality::High,
        Quality::VeryHigh,
    ];
    for quality in qualities {
        for rates in rates {
            for len in lengths {
                let input = noise(len, 3);
                let output: Vec<f32> = oneshot(rates, spec(quality), 1, &input);
                assert_eq!(
                    output.len(),
                    total_output(len, rates),
                    "{quality:?} {rates:?} {len}"
                );
            }
        }
    }
    let boundary: Vec<f32> = oneshot(
        (48_000.0, 44_100.0),
        spec(Quality::High),
        1,
        &noise(50_000, 1),
    );
    assert_eq!(boundary.len(), 45_937);
    let back: Vec<f32> = oneshot(
        (44_100.0, 48_000.0),
        spec(Quality::High),
        1,
        &noise(45_937, 1),
    );
    assert_eq!(back.len(), 49_999);
}

/// The delay at every step, then the same output after clearing
#[test]
fn the_delay_is_exact_and_clearing_starts_again() {
    for rates in [
        (44_100.0, 48_000.0),
        (96_000.0, 44_100.0),
        (44_100.0, 50_000.123),
    ] {
        let input = noise(100_000, 5);
        let mut resampler = Resampler::<f32, f32>::new(rates.0, rates.1, 1).unwrap();
        let ratio = rates.0 / rates.1;
        let mut random = Random::new(9);
        let mut output = Vec::new();
        let (mut taken, mut given) = (0usize, 0usize);
        let mut rest = &input[..];
        while !rest.is_empty() {
            let offered = random.len(3000).min(rest.len());
            let mut room = vec![0.0f32; random.len(3000)];
            let progress = resampler.resample(&rest[..offered], &mut room[..]).unwrap();
            taken += progress.input_frames;
            given += progress.output_frames;
            rest = &rest[progress.input_frames..];
            output.extend_from_slice(&room[..progress.output_frames]);
            assert_eq!(
                resampler.delay(),
                taken as f64 / ratio - given as f64,
                "{rates:?}"
            );
        }
        let total = total_output(input.len(), rates);
        loop {
            let mut room = vec![0.0f32; random.len(3000)];
            let written = resampler.finish(&mut room[..]).unwrap();
            given += written;
            output.extend_from_slice(&room[..written]);
            assert_eq!(resampler.delay(), (total - given) as f64, "{rates:?}");
            if written == 0 {
                break;
            }
        }
        assert_eq!(output.len(), total);

        resampler.clear();
        assert_eq!(resampler.delay(), 0.0);
        let again = streamed(&mut resampler, &input, 21, 777, 1234);
        assert!(
            again
                .iter()
                .zip(&output)
                .all(|(a, b)| a.to_bits() == b.to_bits())
        );
        assert_eq!(again.len(), output.len());
    }
}

/// One input sample is 1/ρ of output still to come
#[test]
fn one_sample_in_is_a_delay_of_the_inverse_ratio() {
    let mut resampler = Resampler::<f32, f32>::new(44_100.0, 48_000.0, 1).unwrap();
    resampler
        .resample(&[0.5f32][..], &mut [0.0f32; 4][..])
        .unwrap();
    assert_eq!(resampler.delay(), 48_000.0 / 44_100.0);
}

#[test]
fn input_is_taken_as_far_as_the_output_has_room() {
    let cases = [
        ((1.0, 2.0), 100, 50),
        ((2.0, 1.0), 100, 200),
        ((2.0, 5.0), 10_000, 4000),
    ];
    for (rates, room, expected) in cases {
        let mut resampler = Resampler::<f32, f32>::new(rates.0, rates.1, 1).unwrap();
        let input = vec![0.0f32; 10_000];
        let mut output = vec![0.0f32; room];
        let progress = resampler.resample(&input[..], &mut output[..]).unwrap();
        assert_eq!(progress.input_frames, expected, "{rates:?}");
    }
}

#[test]
fn the_end_drains_and_can_repeat() {
    let mut resampler = Resampler::<f32, f32>::new(1.0, 2.0, 1).unwrap();
    let input = noise(1000, 2);
    let mut output = vec![0.0f32; 1000];
    let progress = resampler.resample(&input[..], &mut output[..]).unwrap();
    assert_eq!(progress.input_frames, 500);
    let mut room = vec![0.0f32; 4000];
    let progress = resampler.resample(&input[500..], &mut room[..]).unwrap();
    assert_eq!(progress.input_frames, 500);
    let given = resampler.finish(&mut [0.0f32; 0][..]).unwrap();
    assert_eq!(given, 0);
    let rest = resampler.finish(&mut room[..]).unwrap();
    assert!(resampler.delay() == 0.0);
    let total = resampler.finish(&mut room[..]).unwrap();
    assert_eq!(total, 0);
    assert!(rest > 0);
    assert_eq!(resampler.finish(&mut [0.0f32; 8][..]), Ok(0));
}

/// It leaves the resampler working, where libsoxr crashed
#[test]
fn input_after_the_end_is_an_error() {
    let mut resampler = Resampler::<f32, f32>::new(44_100.0, 48_000.0, 1).unwrap();
    resampler.finish(&mut [0.0f32; 0][..]).unwrap();
    let error = resampler.resample(&[0.0f32; 10][..], &mut [0.0f32; 10][..]);
    assert_eq!(error, Err(Error::InputAfterEnd));
    assert_eq!(resampler.error(), None);
    assert!(
        resampler
            .resample(&[0.0f32; 0][..], &mut [0.0f32; 10][..])
            .is_ok()
    );
    resampler.clear();
    assert!(
        resampler
            .resample(&[0.0f32; 10][..], &mut [0.0f32; 10][..])
            .is_ok()
    );
}

#[test]
fn a_constant_ratio_takes_only_itself_again() {
    let mut resampler = Resampler::<f32, f32>::new(2.0, 1.0, 1).unwrap();
    for delta in [0.0, 4e-16, 8e-16, 1e-15] {
        assert_eq!(resampler.set_io_ratio(2.0 + delta, 0), Ok(()), "{delta}");
    }
    assert_eq!(resampler.set_io_ratio(2.0, 100), Ok(()));
    for delta in [1.5e-15, 2e-15, 1e-14, 0.1] {
        assert_eq!(
            resampler.set_io_ratio(2.0 + delta, 0),
            Err(Error::VaryingRatio),
            "{delta}"
        );
    }
    assert_eq!(resampler.set_io_ratio(1.5, 0), Err(Error::VaryingRatio));
    assert_eq!(
        resampler.set_io_ratio(0.0, 0),
        Err(Error::IoRatioOutOfRange)
    );
    assert_eq!(
        resampler.set_io_ratio(-1.0, 0),
        Err(Error::IoRatioOutOfRange)
    );
    // Returned, not kept
    assert_eq!(resampler.error(), None);
    assert!(
        resampler
            .resample(&[0.0f32; 4][..], &mut [0.0f32; 4][..])
            .is_ok()
    );
}

/// Output frames of 3000 input frames, after 1000 at 2:1, a clear and a
/// ratio of 3, and what setting the ratio returned
fn after_clearing(quality: QualitySpec) -> (Result<(), Error>, usize) {
    let mut resampler = Resampler::<f32, f32>::builder(2.0, 1.0, 1)
        .quality(quality)
        .build()
        .unwrap();
    resampler
        .resample(&noise(1000, 1)[..], &mut vec![0.0f32; 1000][..])
        .unwrap();
    resampler.clear();
    assert_eq!(resampler.delay(), 0.0);
    let set = resampler.set_io_ratio(3.0, 0);
    (
        set,
        streamed(&mut resampler, &noise(3000, 2), 1, 3000, 3000).len(),
    )
}

#[test]
fn clearing_lets_only_the_lsr_presets_take_a_new_ratio() {
    for recipe in 0..16 {
        let quality = QualitySpec::from_bits(recipe, 0);
        let expected = if (8..=10).contains(&recipe) {
            (Ok(()), 1000)
        } else {
            (Err(Error::VaryingRatio), 1500)
        };
        assert_eq!(after_clearing(quality), expected, "{recipe}");
    }
}

/// The flag decides, not the preset
#[test]
fn keeping_the_ratio_on_clearing_follows_its_flag() {
    let mut high = QualitySpec::new(Quality::High);
    high.keep_ratio_on_clear = false;
    assert_eq!(after_clearing(high), (Ok(()), 1000));
    let lsr = QualitySpec::from_bits(8, soxr::bits::KEEP_RATIO_ON_CLEAR);
    assert!(lsr.keep_ratio_on_clear);
    assert_eq!(after_clearing(lsr), (Err(Error::VaryingRatio), 1500));
}

/// A new ratio is for before any resampling, once; with none, the ratio stays
#[test]
fn a_new_ratio_after_clearing_comes_before_resampling() {
    let make = || {
        let mut resampler = Resampler::<f32, f32>::builder(2.0, 1.0, 1)
            .quality(Quality::LsrBest)
            .build()
            .unwrap();
        resampler.clear();
        resampler
    };
    let mut resampler = make();
    let output = streamed(&mut resampler, &noise(3000, 2), 1, 3000, 3000);
    assert_eq!(output.len(), 1500);
    assert_eq!(resampler.set_io_ratio(3.0, 0), Err(Error::VaryingRatio));
    let mut resampler = make();
    assert_eq!(resampler.set_io_ratio(3.0, 0), Ok(()));
    assert_eq!(resampler.io_ratio(), 3.0);
    assert_eq!(resampler.set_io_ratio(4.0, 0), Err(Error::VaryingRatio));
    assert_eq!(resampler.set_io_ratio(3.0, 0), Ok(()));
    let mut resampler = make();
    assert_eq!(resampler.set_io_ratio(2.0, 0), Ok(()));
    assert_eq!(resampler.set_io_ratio(3.0, 0), Err(Error::VaryingRatio));
    let mut resampler = make();
    resampler.finish(&mut [0.0f32; 4][..]).unwrap();
    assert_eq!(resampler.set_io_ratio(3.0, 0), Err(Error::VaryingRatio));
    resampler.clear();
    assert_eq!(resampler.set_io_ratio(3.0, 0), Ok(()));
    resampler.clear();
    // Where libsoxr crashes, the ratio from before the clears
    let output = streamed(&mut resampler, &noise(3000, 2), 1, 3000, 3000);
    assert_eq!(output.len(), 1000);
}

/// A ratio that the quality spec does not allow is an error that stays until
/// a clear
#[test]
fn a_new_ratio_is_checked_against_the_quality() {
    let mut quality = QualitySpec::new(Quality::LsrBest);
    quality.passband_end = 0.97;
    quality.stopband_begin = 1.05;
    let mut resampler = Resampler::<f32, f32>::builder(2.0, 1.0, 1)
        .quality(quality)
        .build()
        .unwrap();
    resampler.clear();
    let error = Err(Error::ImagingGreaterThanRolloff);
    assert_eq!(resampler.set_io_ratio(0.5, 0), error);
    assert_eq!(resampler.error(), Some(Error::ImagingGreaterThanRolloff));
    assert_eq!(resampler.finish(&mut [0.0f32; 4][..]).map(|_| ()), error);
    resampler.clear();
    assert_eq!(resampler.error(), None);
    assert_eq!(resampler.io_ratio(), 2.0);
}

/// At a gain of 1
#[test]
fn a_ratio_of_one_passes_the_input_through() {
    for quality in QUALITIES {
        let input = noise(1000, 4);
        let mut resampler = Resampler::<f32, f32>::builder(48_000.0, 48_000.0, 1)
            .quality(spec(quality))
            .build()
            .unwrap();
        let mut output = vec![0.0f32; 1000];
        let progress = resampler.resample(&input[..], &mut output[..]).unwrap();
        assert_eq!(progress.output_frames, 1000);
        assert_eq!(resampler.delay(), 0.0);
        assert!(
            output
                .iter()
                .zip(&input)
                .all(|(a, b)| a.to_bits() == b.to_bits())
        );
    }
    let input: Vec<f64> = noise(1000, 4).into_iter().map(f64::from).collect();
    let output: Vec<f64> = oneshot((1.0, 1.0), spec(Quality::VeryHigh), 1, &input);
    assert_eq!(output, input);
}

#[test]
fn a_ratio_of_one_applies_the_gain() {
    let input = noise(1000, 4);
    let output: Vec<f32> = Resampler::<f32, f32>::builder(1.0, 1.0, 1)
        .io(IoSpec::new().with_scale(0.5))
        .oneshot(&input[..])
        .unwrap();
    assert!(output.iter().zip(&input).all(|(a, b)| *a == b * 0.5));
    let scaled: Vec<f32> = Resampler::<f32, f32>::builder(1.0, 1.0, 1)
        .io(IoSpec::new().with_scale(0.3))
        .oneshot(&input[..])
        .unwrap();
    assert!(
        scaled
            .iter()
            .zip(&input)
            .all(|(a, b)| *a == (f64::from(*b) * 0.3) as f32)
    );
}

/// As in libsoxr, the last 2 frames wait for the end of the input
#[test]
fn a_ratio_of_one_with_a_gain_holds_two_frames() {
    let input = noise(1000, 5);
    for quality in [Quality::High, Quality::LsrFast] {
        let mut scaled = Resampler::<f32, f32>::builder(1.0, 1.0, 2)
            .quality(spec(quality))
            .io(IoSpec::new().with_scale(0.5))
            .build()
            .unwrap();
        assert_eq!(run(&mut scaled, &input), (998, 2.0, 2), "{quality:?}");
    }
    let mut to_integer = Resampler::<f32, i16>::builder(1.0, 1.0, 1)
        .io(IoSpec::new().with_dither(Dither::None))
        .build()
        .unwrap();
    assert_eq!(run(&mut to_integer, &input), (998, 2.0, 2));
    let integers: Vec<i16> = input.iter().map(|x| (x * 30_000.0) as i16).collect();
    let mut from_integer = Resampler::<i16, f64>::new(1.0, 1.0, 1).unwrap();
    assert_eq!(run(&mut from_integer, &integers), (998, 2.0, 2));
    let mut same = Resampler::<f32, f64>::new(1.0, 1.0, 1).unwrap();
    assert_eq!(run(&mut same, &input), (1000, 0.0, 0));
}

/// Of `input` in every channel: the frames given before its end, the delay
/// then, and the frames given after
fn run<I: Sample, O: Sample>(resampler: &mut Resampler<I, O>, input: &[I]) -> (usize, f64, usize) {
    let channels = resampler.channels();
    let interleaved: Vec<I> = input
        .iter()
        .flat_map(|x| std::iter::repeat_n(*x, channels))
        .collect();
    let mut output = vec![O::default(); interleaved.len()];
    let progress = resampler
        .resample(&interleaved[..], &mut output[..])
        .unwrap();
    let delay = resampler.delay();
    let after_end = resampler.finish(&mut output[..]).unwrap();
    (progress.output_frames, delay, after_end)
}

/// Channels of different layouts and lengths are errors
#[test]
fn buffers_must_match_the_channels() {
    let mut resampler = Resampler::<f32, f32>::new(1.0, 2.0, 2).unwrap();
    let odd = resampler.resample(&[0.0f32; 3][..], &mut [0.0f32; 8][..]);
    assert_eq!(odd, Err(Error::BufferLayout));
    let left = [0.0f32; 4];
    let right = [0.0f32; 3];
    let channels: [&[f32]; 2] = [&left, &right];
    let uneven = resampler.resample(&channels[..], &mut [0.0f32; 8][..]);
    assert_eq!(uneven, Err(Error::BufferLayout));
    let one: [&[f32]; 1] = [&left];
    assert_eq!(
        resampler.resample(&one[..], &mut [0.0f32; 8][..]),
        Err(Error::BufferLayout)
    );
    assert_eq!(
        resampler.finish(&mut [0.0f32; 3][..]),
        Err(Error::BufferLayout)
    );
    let quality = QualitySpec::new(Quality::High);
    assert_eq!(resampler.quality(), &quality);
}

/// The extreme ratios work, not only build
#[test]
fn extreme_ratios_resample() {
    for (rates, len) in [((1.0, 1e6), 3), ((1e6, 1.0), 3_000_000)] {
        let input = vec![0.25f32; len];
        let output: Vec<f32> = oneshot(rates, spec(Quality::High), 1, &input);
        assert_eq!(output.len(), total_output(len, rates));
        // Three samples ring a little over their level once band-limited
        let middle = output[output.len() / 2];
        assert!((0.24..0.27).contains(&middle), "{rates:?}: {middle}");
    }
}

/// One shot is a resampler at LQ given all the input and then the end
#[test]
fn oneshot_is_all_the_input_at_low_quality() {
    let rates = (48_000.0, 44_100.0);
    let input = noise(50_000, 12);
    let once: Vec<f32> = Resampler::<f32, f32>::builder(rates.0, rates.1, 1)
        .oneshot(&input[..])
        .unwrap();
    let mut resampler = Resampler::<f32, f32>::builder(rates.0, rates.1, 1)
        .quality(Quality::Low)
        .build()
        .unwrap();
    let mut output = vec![0.0f32; 50_000];
    let progress = resampler.resample(&input[..], &mut output[..]).unwrap();
    assert_eq!(progress.input_frames, 50_000);
    let rest = resampler
        .finish(&mut output[progress.output_frames..])
        .unwrap();
    output.truncate(progress.output_frames + rest);
    assert_eq!(output.len(), 45_937);
    assert!(
        output.len() == once.len()
            && output
                .iter()
                .zip(&once)
                .all(|(a, b)| a.to_bits() == b.to_bits())
    );
}
