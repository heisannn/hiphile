//! The quality spec: presets, recipes, the order of the checks and their
//! messages, and the engine each spec takes

use soxr::{
    Dither, Engine, Error, IoSpec, Phase, Quality, QualitySpec, Recipe, Resampler, Rolloff, bits,
};

const UP: (f64, f64) = (44_100.0, 48_000.0);
const DOWN: (f64, f64) = (48_000.0, 44_100.0);

fn spec(recipe: u32) -> QualitySpec {
    QualitySpec::from_bits(recipe, 0)
}

fn build(rates: (f64, f64), quality: QualitySpec) -> Result<Engine, Error> {
    Resampler::<f32, f32>::builder(rates.0, rates.1, 1)
        .quality(quality)
        .build()
        .map(|resampler| resampler.engine())
}

fn changed(mut quality: QualitySpec, change: impl FnOnce(&mut QualitySpec)) -> QualitySpec {
    change(&mut quality);
    quality
}

fn band(passband_end: f64, stopband_begin: f64) -> QualitySpec {
    let mut quality = spec(bits::HQ);
    quality.passband_end = passband_end;
    quality.stopband_begin = stopband_begin;
    quality
}

/// `soxr_quality_spec(recipe, 0)` as libsoxr returns it, to the bit
#[test]
fn presets_are_libsoxrs_to_the_bit() {
    #[allow(clippy::excessive_precision)]
    let expected: [(u32, f64, f64, Rolloff); 11] = [
        (0, 0.0, 0.93886648602569966, Rolloff::Medium),
        (1, 16.0, 0.67626953125, Rolloff::Medium),
        (2, 16.0, 0.9158810491765671, Rolloff::Medium),
        (3, 16.0, 0.9158810491765671, Rolloff::Small),
        (4, 20.0, 0.91374365327330664, Rolloff::Small),
        (5, 24.0, 0.91232195381988845, Rolloff::Small),
        (6, 28.0, 0.91132831989516627, Rolloff::Small),
        (7, 32.0, 0.91050184934434353, Rolloff::Small),
        (8, 23.0, 0.9309999942779541, Rolloff::Small),
        (9, 19.0, 0.8320000171661377, Rolloff::Small),
        // The rolloff bits of 3 are taken as none
        (10, 15.0, 0.66299998760223389, Rolloff::None),
    ];
    for (recipe, precision, passband_end, rolloff) in expected {
        let quality = spec(recipe);
        assert_eq!(quality.precision.to_bits(), precision.to_bits(), "{recipe}");
        assert_eq!(
            quality.passband_end.to_bits(),
            passband_end.to_bits(),
            "{recipe}"
        );
        assert_eq!(
            quality.phase_response.to_bits(),
            50.0f64.to_bits(),
            "{recipe}"
        );
        assert_eq!(
            quality.stopband_begin.to_bits(),
            1.0f64.to_bits(),
            "{recipe}"
        );
        assert_eq!(quality.rolloff, rolloff, "{recipe}");
        assert!(!quality.variable_rate && !quality.double_precision);
        assert!(!quality.high_precision_clock);
    }
    assert_eq!(QualitySpec::default(), spec(bits::HQ));
    assert_eq!(QualitySpec::from(Quality::VeryHigh), spec(bits::VHQ));
}

#[test]
fn steep_presets_are_libsoxrs_to_the_bit() {
    #[allow(clippy::excessive_precision)]
    let expected: [(u32, f64); 11] = [
        (0, 0.98777329720513996),
        (1, 0.9831762098353134),
        (2, 0.9831762098353134),
        (3, 0.9831762098353134),
        (4, 0.98274873065466128),
        (5, 0.98246439076397774),
        (6, 0.98226566397903325),
        (7, 0.98210036986886873),
        (8, 0.9825253148143992),
        (9, 0.98283977491453212),
        (10, 0.98331294043370743),
    ];
    for (recipe, passband_end) in expected {
        let quality = spec(recipe | bits::STEEP_FILTER);
        assert_eq!(
            quality.passband_end.to_bits(),
            passband_end.to_bits(),
            "{recipe}"
        );
        assert_eq!(quality.precision, spec(recipe).precision);
        assert_eq!(quality.rolloff, spec(recipe).rolloff);
    }
}

/// 11 and 12 are QQ, 13 to 15 VHQ, and higher bits are ignored
#[test]
fn spare_quality_numbers_map_as_libsoxr_maps_them() {
    for recipe in [11, 12] {
        assert_eq!(spec(recipe), spec(bits::QQ));
    }
    for recipe in 13..=15 {
        assert_eq!(spec(recipe), spec(bits::VHQ));
    }
    let recipe = Recipe::from_bits(99);
    assert_eq!(recipe.quality, Quality::Bits16);
    assert_eq!(recipe.phase, Phase::Maximum);
    assert!(recipe.steep);
    assert_eq!(build(UP, spec(99)), Ok(Engine::Single));
}

#[test]
fn phase_bits_set_the_phase_response() {
    let expected = [
        (bits::LINEAR_PHASE, 50.0),
        (bits::INTERMEDIATE_PHASE, 25.0),
        (bits::MAXIMUM_PHASE, 100.0),
        (bits::MINIMUM_PHASE, 0.0),
    ];
    for (phase, response) in expected {
        for quality in 0..16 {
            let with_phase = spec(quality | phase);
            assert_eq!(with_phase.phase_response, response);
            assert_eq!(spec(quality).phase_response, 50.0);
            assert_eq!(
                changed(spec(quality), |quality| quality.phase_response = response),
                with_phase
            );
        }
    }
    let recipe = Recipe::new(Quality::High)
        .with_phase(Phase::Minimum)
        .steep(true);
    assert_eq!(
        recipe,
        Recipe::from_bits(bits::HQ | bits::MINIMUM_PHASE | bits::STEEP_FILTER)
    );
}

#[test]
fn flags_join_the_presets() {
    let quality = QualitySpec::from_bits(bits::HQ, bits::VR | bits::HI_PREC_CLOCK);
    assert!(quality.variable_rate && quality.high_precision_clock);
    assert!(!quality.double_precision);
    assert_eq!(quality.rolloff, Rolloff::Small);

    let rolloffs = [
        (bits::HQ, bits::ROLLOFF_MEDIUM, Rolloff::Medium),
        (bits::HQ, bits::ROLLOFF_NONE, Rolloff::None),
        (bits::LQ, bits::ROLLOFF_SMALL, Rolloff::Medium),
        // Medium and none together are bits 3
        (bits::LQ, bits::ROLLOFF_NONE, Rolloff::None),
    ];
    for (recipe, flags, rolloff) in rolloffs {
        assert_eq!(QualitySpec::from_bits(recipe, flags).rolloff, rolloff);
    }
    assert!(QualitySpec::from_bits(bits::QQ, bits::DOUBLE_PRECISION).double_precision);
}

#[test]
fn the_spec_chooses_the_engine() {
    let with = |recipe: u32, flags: u32| build(UP, QualitySpec::from_bits(recipe, flags));
    assert_eq!(with(bits::HQ, 0), Ok(Engine::Single));
    assert_eq!(with(bits::QQ, 0), Ok(Engine::Single));
    assert_eq!(with(bits::VHQ, 0), Ok(Engine::Double));
    assert_eq!(with(bits::HQ, bits::DOUBLE_PRECISION), Ok(Engine::Double));
    assert_eq!(with(bits::HQ, bits::VR), Ok(Engine::Variable));
    assert_eq!(with(bits::VHQ, bits::VR), Ok(Engine::Variable));
    for (precision, engine) in [
        (15.0, Engine::Single),
        (21.0, Engine::Double),
        (33.0, Engine::Double),
    ] {
        let quality = changed(spec(bits::HQ), |quality| quality.precision = precision);
        assert_eq!(build(UP, quality), Ok(engine), "{precision}");
    }
    let names = [Engine::Single, Engine::Double, Engine::Variable].map(Engine::name);
    assert_eq!(names, ["cr32", "cr64", "vr32"]);
}

#[test]
fn rates_must_be_positive_and_channels_some() {
    let make = |input: f64, output: f64, channels: usize| {
        Resampler::<f32, f32>::new(input, output, channels).map(|_| ())
    };
    assert_eq!(make(44_100.0, 48_000.0, 2), Ok(()));
    assert_eq!(make(1.0, 1.0, 1), Ok(()));
    assert_eq!(make(1.0, 1e6, 1), Ok(()));
    assert_eq!(make(1e6, 1.0, 1), Ok(()));
    for (input, output) in [
        (0.0, 1.0),
        (1.0, 0.0),
        (-1.0, 1.0),
        (0.0, 0.0),
        (f64::NAN, 1.0),
    ] {
        assert_eq!(make(input, output, 1), Err(Error::IoRatioOutOfRange));
    }
    assert_eq!(make(f64::INFINITY, 1.0, 1), Err(Error::IoRatioOutOfRange));
    assert_eq!(make(1e300, 1e-300, 1), Err(Error::IoRatioOutOfRange));
    assert_eq!(make(44_100.0, 48_000.0, 0), Err(Error::InvalidChannels));
    // The rates come first
    assert_eq!(make(0.0, 48_000.0, 0), Err(Error::IoRatioOutOfRange));
}

#[test]
fn bad_quality_values_are_the_errors_libsoxr_gives() {
    let with_precision =
        |precision: f64| changed(spec(bits::HQ), |quality| quality.precision = precision);
    let with_phase = |phase_response: f64| {
        changed(spec(bits::HQ), |quality| {
            quality.phase_response = phase_response
        })
    };
    let cases = [
        (UP, with_precision(14.0), Err(Error::Precision)),
        (UP, with_precision(33.5), Err(Error::Precision)),
        (UP, with_precision(15.0), Ok(Engine::Single)),
        (UP, with_phase(-1.0), Err(Error::PhaseResponse)),
        (UP, with_phase(101.0), Err(Error::PhaseResponse)),
        (UP, band(0.49, 1.0), Err(Error::TransitionBandwidth)),
        (UP, band(0.5, 0.9), Ok(Engine::Single)),
        (UP, band(0.999, 1.0), Err(Error::TransitionBandwidth)),
        (UP, band(0.998, 1.0), Ok(Engine::Single)),
        // Upsampling would fail on imaging first
        (DOWN, band(0.9, 1.41), Err(Error::TransitionBandwidth)),
        (DOWN, band(1.0, 1.49), Ok(Engine::Single)),
        (UP, band(1.0, 1.49), Err(Error::ImagingGreaterThanRolloff)),
        (DOWN, band(1.0, 1.51), Err(Error::TransitionBandwidth)),
        (UP, band(0.9, 1.05), Ok(Engine::Single)),
        (DOWN, band(0.9, 1.05), Ok(Engine::Single)),
        (UP, band(91.3, 100.0), Ok(Engine::Single)),
    ];
    for (rates, quality, expected) in cases {
        assert_eq!(build(rates, quality), expected, "{rates:?} {quality:?}");
    }
}

/// The limits themselves pass, and a little past them fails
#[test]
fn limits_of_the_band_are_as_libsoxrs() {
    let cases = [
        (UP, band(0.9, 1.1), Ok(Engine::Single)),
        (UP, band(0.9, 1.1001), Err(Error::ImagingGreaterThanRolloff)),
        (UP, band(0.9, 1.099), Ok(Engine::Single)),
        (DOWN, band(0.9, 1.1001), Ok(Engine::Single)),
        (DOWN, band(0.9, 0.9019), Err(Error::TransitionBandwidth)),
        (DOWN, band(0.9, 0.90199999), Ok(Engine::Single)),
        (DOWN, band(0.9, 1.4), Ok(Engine::Single)),
        (DOWN, band(0.9, 1.40001), Err(Error::TransitionBandwidth)),
        (DOWN, band(0.45, 0.9), Err(Error::TransitionBand)),
        (DOWN, band(0.4999, 0.9), Err(Error::TransitionBand)),
        (DOWN, band(0.49999, 0.9), Err(Error::TransitionBand)),
        (DOWN, band(1.2, 1.5), Ok(Engine::Single)),
        (DOWN, band(1.2, 1.50001), Ok(Engine::Single)),
        (DOWN, band(1.2, 1.5001), Err(Error::TransitionBand)),
        (DOWN, band(1.2, 1.55), Err(Error::TransitionBand)),
    ];
    for (rates, quality, expected) in cases {
        assert_eq!(build(rates, quality), expected, "{rates:?} {quality:?}");
    }
    let precision =
        |precision: f64| changed(spec(bits::HQ), |quality| quality.precision = precision);
    assert_eq!(build(UP, precision(14.99)), Err(Error::Precision));
    assert_eq!(build(UP, precision(33.0001)), Err(Error::Precision));
    for phase_response in [0.0, 100.0] {
        let quality = changed(spec(bits::HQ), |quality| {
            quality.phase_response = phase_response
        });
        assert_eq!(build(UP, quality), Ok(Engine::Single));
    }
    let quick = changed(spec(bits::QQ), |quality| {
        quality.passband_end = 0.45;
        quality.stopband_begin = 0.9;
    });
    assert_eq!(build(UP, quick), Err(Error::TransitionBand));
}

/// As far as this crate's tolerances follow libsoxr's
#[test]
fn tolerances_past_the_limits() {
    let past = |limit: f64, relative: f64| limit * (1.0 + relative);
    let below = |limit: f64, relative: f64| limit * (1.0 - relative);
    let cases = [
        (
            DOWN,
            band(0.9, 0.9 + below(0.002, 9e-6)),
            Ok(Engine::Single),
        ),
        (
            DOWN,
            band(0.9, 0.9 + below(0.002, 2e-5)),
            Err(Error::TransitionBandwidth),
        ),
        (DOWN, band(0.9, 0.9 + past(0.5, 9e-6)), Ok(Engine::Single)),
        (
            DOWN,
            band(0.9, 0.9 + past(0.5, 1.1e-5)),
            Err(Error::TransitionBandwidth),
        ),
        (DOWN, band(below(0.5, 9e-6), 0.9), Ok(Engine::Single)),
        (
            DOWN,
            band(below(0.5, 1e-5 * 1.1), 0.9),
            Err(Error::TransitionBand),
        ),
        (DOWN, band(1.2, past(1.5, 9e-6)), Ok(Engine::Single)),
        (
            DOWN,
            band(1.2, past(1.5, 1.1e-5)),
            Err(Error::TransitionBand),
        ),
        (UP, band(0.9, 1.0 + past(0.1, 4e-5)), Ok(Engine::Single)),
        (
            UP,
            band(0.9, 1.0 + past(0.1, 1e-4)),
            Err(Error::ImagingGreaterThanRolloff),
        ),
    ];
    for (rates, quality, expected) in cases {
        assert_eq!(build(rates, quality), expected, "{rates:?} {quality:?}");
    }
    let precision =
        |precision: f64| changed(spec(bits::HQ), |quality| quality.precision = precision);
    assert_eq!(
        build(UP, precision(below(15.0, 2e-6))),
        Err(Error::Precision)
    );
    assert_eq!(
        build(UP, precision(past(33.0, 2e-6))),
        Err(Error::Precision)
    );
    let phase = |phase_response: f64| {
        changed(spec(bits::HQ), |quality| {
            quality.phase_response = phase_response
        })
    };
    assert_eq!(build(UP, phase(-2e-6)), Err(Error::PhaseResponse));
    assert_eq!(
        build(UP, phase(past(100.0, 2e-6))),
        Err(Error::PhaseResponse)
    );
}

/// In libsoxr's order: imaging, then the transition's width, then its
/// position, then precision, then phase
#[test]
fn the_first_failed_check_is_the_error() {
    let mut quality = band(1.0, 1.49);
    quality.precision = 14.0;
    assert_eq!(build(UP, quality), Err(Error::ImagingGreaterThanRolloff));
    let mut quality = band(0.9, 0.9019);
    quality.precision = 14.0;
    assert_eq!(build(DOWN, quality), Err(Error::TransitionBandwidth));
    let mut quality = band(0.45, 0.9);
    quality.precision = 14.0;
    quality.phase_response = 101.0;
    assert_eq!(build(DOWN, quality), Err(Error::TransitionBand));
    let mut quality = spec(bits::HQ);
    quality.precision = 14.0;
    quality.phase_response = 101.0;
    assert_eq!(build(DOWN, quality), Err(Error::Precision));
    // Both the width and the position are wrong
    assert_eq!(
        build(DOWN, band(0.49, 1.0)),
        Err(Error::TransitionBandwidth)
    );
}

#[test]
fn variable_rate_skips_the_checks() {
    for mut quality in [band(0.45, 1.0), band(1.0, 1.49), spec(bits::HQ)] {
        quality.precision = 14.0;
        quality.phase_response = 101.0;
        quality.variable_rate = true;
        assert_eq!(build(UP, quality), Ok(Engine::Variable));
    }
}

/// Percentages of the API before libsoxr 0.1.1
#[test]
fn percentages_are_converted() {
    // Divided, so one ulp off the nearest double to 0.913
    assert_eq!(band(91.3, 100.0).band(), (91.3 / 100.0, 1.0));
    assert_eq!(band(91.3, 100.0).band().0.to_bits() + 1, 0.913f64.to_bits());
    assert_eq!(band(91.3, 95.0).band(), (91.3 / 100.0, 2.0 - 0.95));
    assert_eq!(band(91.3, 95.0).band().1, 1.05);
    assert_eq!(band(0.913, 1.05).band(), (0.913, 1.05));
    assert_eq!(
        build(DOWN, band(95.0, 110.0)),
        Err(Error::TransitionBandwidth)
    );
    assert_eq!(
        build(DOWN, band(0.95, 0.9)),
        Err(Error::TransitionBandwidth)
    );
}

#[test]
fn messages_are_libsoxrs() {
    let messages = [
        (
            Error::ImagingGreaterThanRolloff,
            "imaging greater than rolloff",
        ),
        (
            Error::TransitionBandwidth,
            "transition bandwidth not in [0.2,50] % of nyquist",
        ),
        (
            Error::TransitionBand,
            "transition band not within [50,150] % of nyquist",
        ),
        (Error::Precision, "precision not in [15,33] bits"),
        (
            Error::PhaseResponse,
            "phase response not in [0=min-phase,100=max-phase] %",
        ),
        (Error::IoRatioOutOfRange, "I/O ratio out-of-range"),
        (Error::InvalidChannels, "invalid # of channels"),
        (
            Error::VaryingRatio,
            "varying O/I ratio is not supported with this quality level",
        ),
        (Error::InputFailure, "input function reported failure"),
    ];
    for (error, message) in messages {
        assert_eq!(error.to_string(), message);
    }
}

#[test]
fn io_spec_defaults() {
    let io = IoSpec::default();
    assert_eq!(io.scale, 1.0);
    assert_eq!(io.dither, Dither::Tpdf { seed: None });
    let resampler = Resampler::<f32, f32>::new(44_100.0, 48_000.0, 1).unwrap();
    assert_eq!(resampler.io(), &io);
    assert_eq!(resampler.quality(), &spec(bits::HQ));
    assert_eq!(resampler.io_ratio(), 44_100.0 / 48_000.0);
    let unscaled = Resampler::<f32, f32>::builder(1.0, 2.0, 1)
        .io(IoSpec::new().with_scale(f64::NAN))
        .build();
    assert_eq!(unscaled.err(), Some(Error::Scale));
}
