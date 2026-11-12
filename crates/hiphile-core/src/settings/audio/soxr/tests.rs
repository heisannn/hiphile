use config::{
    ResamplingQuality, SoxrInterpolation, SoxrParameters, SoxrPerformance, SoxrPhase, SoxrQuality,
};
use executor::testing::TestExecutor;

use crate::settings::audio::tests::{settings_saving_to, told};
use crate::settings::{AudioSnapshot, Choice, SoxrSetting};

const SOXR: usize = 2;

fn soxr_of(settings: &AudioSnapshot) -> audio::SoxrSettings {
    let audio::Resampling::Soxr(resampling) = settings.output().resampling() else {
        panic!("expected soxr");
    };
    resampling
}

fn labels(options: &[Choice]) -> Vec<&str> {
    options.iter().map(Choice::label).collect()
}

#[test]
fn soxr_is_told_saved_and_reaches_the_output() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);

    settings.select_resampler(SOXR);
    cx.run_until_parked();

    assert_eq!(told(&settings), 1);
    assert_eq!(
        store.resampling_quality().unwrap(),
        ResamplingQuality::Soxr(SoxrParameters::default())
    );
    {
        let settings = settings.snapshot();
        assert_eq!(settings.resampler_ix(), SOXR);
        assert!(settings.uses_soxr());
        assert!(!settings.uses_r8brain());
        assert_eq!(settings.soxr_quality_ix(), 3);
        assert_eq!(soxr_of(&settings), audio::SoxrSettings::default());
    }
}

#[test]
fn each_of_the_three_comes_back_at_the_quality_it_had() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.select_rubato_quality(1);
    settings.select_resampler(1);
    settings.select_r8brain_quality(0);
    settings.select_resampler(SOXR);
    settings.select_soxr_quality(1);
    settings.select_resampler(0);
    cx.run_until_parked();
    assert_eq!(store.resampling_quality().unwrap(), ResamplingQuality::High);

    settings.select_resampler(1);
    assert_eq!(settings.snapshot().r8brain_quality_ix(), 0);
    settings.select_resampler(SOXR);
    {
        let settings = settings.snapshot();
        assert_eq!(settings.soxr_quality_ix(), 1);
        assert_eq!(settings.rubato_quality_ix(), 1);
    }
}

#[test]
fn a_changed_value_is_told_saved_and_reaches_the_output() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.select_resampler(SOXR);

    settings.set_soxr_passband_end("0.95");
    settings.set_soxr_precision("32");
    settings.select_soxr_phase(1);
    cx.run_until_parked();

    assert_eq!(told(&settings), 4);
    let saved = store.soxr_parameters().unwrap().quality.filter();
    assert_eq!(saved.passband_end, 0.95);
    assert_eq!(saved.precision, 32.0);
    assert_eq!(saved.phase, SoxrPhase::Intermediate);
    {
        let settings = settings.snapshot();
        let quality = soxr_of(&settings).quality;
        assert_eq!(quality.passband_end, 0.95);
        assert_eq!(quality.precision, 32.0);
        assert_eq!(quality.phase_response, 25.0);
        assert_eq!(settings.soxr_quality_ix(), 4);
    }
}

#[test]
fn the_values_come_back_after_another_resampler() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.select_resampler(SOXR);
    settings.select_soxr_phase(2);

    settings.select_rubato_quality(2);
    cx.run_until_parked();
    assert_eq!(store.resampling_quality().unwrap(), ResamplingQuality::Max);
    assert_eq!(
        store.soxr_parameters().unwrap().quality.filter().phase,
        SoxrPhase::Minimum
    );

    settings.select_resampler(SOXR);
    cx.run_until_parked();
    let ResamplingQuality::Soxr(parameters) = store.resampling_quality().unwrap() else {
        panic!("expected soxr");
    };
    assert_eq!(parameters.quality.filter().phase, SoxrPhase::Minimum);
}

#[test]
fn a_change_that_comes_after_another_resampler_leaves_that_one() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.select_resampler(SOXR);
    settings.select_resampler(1);
    cx.run_until_parked();
    assert_eq!(told(&settings), 2);

    settings.set_soxr_coef_table("1600");
    cx.run_until_parked();

    assert_eq!(told(&settings), 2);
    assert!(matches!(
        store.resampling_quality().unwrap(),
        ResamplingQuality::R8brain(_)
    ));
    assert_eq!(
        store.soxr_parameters().unwrap().performance.coef_table_kb,
        1600
    );
}

#[test]
fn a_preset_is_told_saved_and_reaches_the_output() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.select_resampler(SOXR);

    settings.select_soxr_quality(2);
    cx.run_until_parked();

    assert_eq!(told(&settings), 2);
    assert_eq!(store.soxr_parameters().unwrap().quality, SoxrQuality::High);
    {
        let settings = settings.snapshot();
        assert_eq!(settings.soxr_quality_ix(), 2);
        let quality = soxr_of(&settings).quality;
        assert_eq!(quality.precision, 20.0);
        assert_eq!(quality.engine().name(), "cr32");
    }
}

#[test]
fn another_value_makes_the_preset_custom_and_the_same_one_does_not() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.select_resampler(SOXR);
    settings.select_soxr_quality(1);
    settings.set_soxr_precision("16");
    assert_eq!(settings.snapshot().soxr_quality_ix(), 1);

    settings.set_soxr_precision("24");
    cx.run_until_parked();

    {
        let settings = settings.snapshot();
        assert_eq!(settings.soxr_quality_ix(), 4);
        assert_eq!(
            settings.soxr_filter().passband_end,
            SoxrQuality::Medium.filter().passband_end
        );
    }
    let saved = store.soxr_parameters().unwrap().quality;
    assert!(matches!(saved, SoxrQuality::Custom(_)));
    assert_eq!(saved.filter().precision, 24.0);
}

/// Low's rolloff is not High's in libsoxr, and changes nothing here.
#[test]
fn choosing_custom_keeps_the_sound_and_a_preset_replaces_its_values() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.select_resampler(SOXR);
    settings.select_soxr_quality(0);
    cx.run_until_parked();
    let told_before = told(&settings);

    settings.select_soxr_quality(4);
    cx.run_until_parked();
    assert_eq!(told(&settings), told_before);
    {
        let settings = settings.snapshot();
        assert_eq!(settings.soxr_quality_ix(), 4);
        assert_eq!(settings.soxr_filter(), SoxrQuality::Low.filter());
    }

    settings.select_soxr_quality(3);
    cx.run_until_parked();
    assert_eq!(
        settings.snapshot().soxr_filter(),
        SoxrQuality::VeryHigh.filter()
    );
    let text = std::fs::read_to_string(store.config_path()).unwrap();
    assert!(!text.contains("soxr_precision"), "{text}");
}

#[test]
fn a_preset_leaves_what_does_not_change_the_sound_alone() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.select_resampler(SOXR);
    settings.select_soxr_interpolation(2);
    settings.set_soxr_min_dft("4096");
    settings.set_soxr_large_dft("1048576");
    {
        let settings = settings.snapshot();
        assert_eq!(settings.soxr_quality_ix(), 3);
        let runtime = soxr_of(&settings).runtime;
        assert_ne!(
            runtime.coef_interpolation,
            audio::SoxrSettings::default().runtime.coef_interpolation
        );
        assert_eq!(runtime.log2_min_dft_size, 12);
        assert_eq!(runtime.log2_large_dft_size, 20);
    }

    settings.select_soxr_quality(0);
    {
        let settings = settings.snapshot();
        let performance = settings.soxr_parameters().performance;
        assert_eq!(performance.coef_interpolation, SoxrInterpolation::Cubic);
        assert_eq!(performance.min_dft_size, 4096);
        assert_eq!(settings.soxr_interpolation_ix(), 2);
        assert_eq!(settings.soxr_min_dft_value(), "4096");
        assert_eq!(settings.soxr_large_dft_value(), "1048576");
    }
}

#[test]
fn values_off_the_lists_stay_options_and_presets_read_apart() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.select_resampler(SOXR);
    {
        let settings = settings.snapshot();
        assert_eq!(
            labels(&settings.soxr_precision_options()),
            ["16-bit", "20-bit", "24-bit", "28-bit", "32-bit"]
        );
        assert_eq!(
            labels(&settings.soxr_passband_end_options()),
            ["0.90", "0.911", "0.95", "0.98", "0.99"]
        );
        assert_eq!(settings.soxr_precision_value(), "28");
        assert_eq!(settings.soxr_coef_table_value(), "400");
        assert_eq!(
            labels(&settings.soxr_coef_table_options()),
            ["100 kB", "200 kB", "400 kB", "800 kB", "1600 kB", "3200 kB"]
        );
    }

    let chosen = |options: Vec<Choice>, value: String| {
        let choice = options.into_iter().find(|of| of.value() == value).unwrap();
        choice.label().to_owned()
    };
    let presets: Vec<String> = (0..4)
        .map(|ix| {
            settings.select_soxr_quality(ix);
            let settings = settings.snapshot();
            chosen(
                settings.soxr_passband_end_options(),
                settings.soxr_passband_end_value(),
            )
        })
        .collect();
    assert_eq!(presets, ["0.676", "0.916", "0.914", "0.911"]);

    settings.set_soxr_precision("22.5");
    settings.set_soxr_coef_table("1");
    let settings = settings.snapshot();
    assert_eq!(
        labels(&settings.soxr_precision_options()),
        ["16-bit", "20-bit", "22.5-bit", "24-bit", "28-bit", "32-bit"]
    );
    assert_eq!(
        labels(&settings.soxr_coef_table_options()).first(),
        Some(&"1 kB")
    );
}

#[test]
fn a_phase_and_an_interpolation_are_chosen_by_their_places() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.select_resampler(SOXR);
    for (ix, phase) in [
        (2, SoxrPhase::Minimum),
        (1, SoxrPhase::Intermediate),
        (0, SoxrPhase::Linear),
    ] {
        settings.select_soxr_phase(ix);
        let snapshot = settings.snapshot();
        assert_eq!(snapshot.soxr_phase_ix(), ix);
        assert_eq!(snapshot.soxr_filter().phase, phase);
    }
    for (ix, interpolation) in [
        (1, SoxrInterpolation::Linear),
        (2, SoxrInterpolation::Cubic),
        (0, SoxrInterpolation::Auto),
    ] {
        settings.select_soxr_interpolation(ix);
        let snapshot = settings.snapshot();
        assert_eq!(snapshot.soxr_interpolation_ix(), ix);
        assert_eq!(
            snapshot.soxr_parameters().performance.coef_interpolation,
            interpolation
        );
    }
}

#[test]
fn what_has_a_default_of_its_own_goes_back_to_it() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.select_resampler(SOXR);
    let all = [
        SoxrSetting::Quality,
        SoxrSetting::Precision,
        SoxrSetting::PassbandEnd,
        SoxrSetting::Phase,
        SoxrSetting::DoublePrecision,
        SoxrSetting::CoefTable,
        SoxrSetting::Interpolation,
        SoxrSetting::MinDft,
        SoxrSetting::LargeDft,
    ];
    assert!(
        !all.iter()
            .any(|setting| settings.snapshot().is_soxr_resettable(*setting))
    );

    settings.select_soxr_phase(2);
    settings.set_soxr_double_precision(true);
    settings.set_soxr_coef_table("1600");
    settings.select_soxr_interpolation(2);
    settings.set_soxr_min_dft("4096");
    settings.set_soxr_large_dft("65536");
    settings.set_soxr_precision("20");
    let snapshot = settings.snapshot();
    for setting in all {
        let is_preset_s = matches!(setting, SoxrSetting::Precision | SoxrSetting::PassbandEnd);
        assert_eq!(
            snapshot.is_soxr_resettable(setting),
            !is_preset_s,
            "{setting:?}"
        );
    }

    for setting in all {
        settings.reset_soxr(setting);
    }
    cx.run_until_parked();

    let parameters = settings.snapshot().soxr_parameters();
    assert_eq!(parameters.performance, SoxrPerformance::default());
    assert_eq!(parameters.quality, SoxrQuality::default());
    assert_eq!(store.soxr_parameters().unwrap(), SoxrParameters::default());
}

#[test]
fn double_precision_makes_the_quality_custom() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.select_resampler(SOXR);

    settings.set_soxr_double_precision(true);

    let snapshot = settings.snapshot();
    assert!(snapshot.soxr_filter().double_precision);
    assert_eq!(snapshot.soxr_quality_ix(), 4);
    assert!(snapshot.is_soxr_resettable(SoxrSetting::Quality));
}
