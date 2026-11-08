use config::{R8brainPhase, R8brainQuality, ResamplingQuality};
use gpui_kit::TestAppContext;

use crate::settings::AudioSnapshot;
use crate::settings::audio::tests::{settings_saving_to, told};

#[gpui_kit::test]
fn a_changed_parameter_is_told_saved_and_reaches_the_output(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.select_resampler(1);

    settings.change_r8brain_filter(|filter| filter.transition_band = 3.5);
    cx.run_until_parked();

    assert_eq!(told(&settings), 2);
    let saved = store.r8brain_parameters().unwrap();
    assert_eq!(saved.quality.filter().transition_band, 3.5);
    {
        let settings = settings.snapshot();
        let mut expected = audio::R8brainSettings::default();
        expected.options.transition_band = 3.5;
        assert_eq!(
            settings.output().resampling(),
            audio::Resampling::R8brain(expected)
        );
    }
}

#[gpui_kit::test]
fn the_parameters_come_back_after_another_quality(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.select_resampler(1);
    settings.change_r8brain_filter(|filter| {
        filter.attenuation = 100.0;
        filter.phase = R8brainPhase::Minimum;
    });

    settings.select_rubato_quality(2);
    cx.run_until_parked();
    assert_eq!(store.resampling_quality().unwrap(), ResamplingQuality::Max);
    assert_eq!(
        store
            .r8brain_parameters()
            .unwrap()
            .quality
            .filter()
            .attenuation,
        100.0
    );

    settings.select_resampler(1);
    cx.run_until_parked();
    let ResamplingQuality::R8brain(parameters) = store.resampling_quality().unwrap() else {
        panic!("expected r8brain");
    };
    let filter = parameters.quality.filter();
    assert_eq!(filter.attenuation, 100.0);
    assert_eq!(filter.phase, R8brainPhase::Minimum);
}

#[gpui_kit::test]
fn a_change_that_comes_after_another_quality_leaves_that_quality(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.select_resampler(1);
    settings.select_rubato_quality(2);
    cx.run_until_parked();
    assert_eq!(told(&settings), 2);

    settings.change_r8brain_filter(|filter| filter.attenuation = 100.0);
    cx.run_until_parked();

    assert_eq!(told(&settings), 2);
    assert_eq!(store.resampling_quality().unwrap(), ResamplingQuality::Max);
    assert_eq!(
        store
            .r8brain_parameters()
            .unwrap()
            .quality
            .filter()
            .attenuation,
        100.0
    );
}

#[gpui_kit::test]
fn a_bit_depth_sets_the_attenuation_and_other_values_read_other(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.select_resampler(1);
    let labels = |settings: &AudioSnapshot| -> Vec<String> {
        settings
            .r8brain_bit_depth_options()
            .iter()
            .map(|choice| choice.value().to_owned())
            .collect()
    };

    {
        let settings = settings.snapshot();
        assert_eq!(settings.r8brain_bit_depth_value(), "24-bit");
        assert!(!labels(&settings).contains(&"Other".to_owned()));
    }

    settings.set_r8brain_bit_depth("16-bit");
    assert_eq!(settings.snapshot().r8brain_filter().attenuation, 136.45);

    settings.change_r8brain_filter(|filter| filter.attenuation = 140.0);
    {
        let settings = settings.snapshot();
        assert_eq!(settings.r8brain_bit_depth_value(), "Other");
        assert!(labels(&settings).contains(&"Other".to_owned()));
    }
}

#[gpui_kit::test]
fn a_phase_is_chosen_by_its_place(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.select_resampler(1);
    assert_eq!(settings.snapshot().r8brain_phase_ix(), 0);

    settings.select_r8brain_phase(1);
    {
        let settings = settings.snapshot();
        assert_eq!(settings.r8brain_phase_ix(), 1);
        assert_eq!(settings.r8brain_filter().phase, R8brainPhase::Minimum);
    }
}

#[gpui_kit::test]
fn a_preset_is_told_saved_and_reaches_the_output(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.select_resampler(1);
    assert_eq!(settings.snapshot().r8brain_quality_ix(), 2);

    settings.select_r8brain_quality(0);
    cx.run_until_parked();

    assert_eq!(told(&settings), 2);
    assert_eq!(
        store.r8brain_parameters().unwrap().quality,
        R8brainQuality::Low
    );
    {
        let settings = settings.snapshot();
        assert_eq!(settings.r8brain_quality_ix(), 0);
        let audio::Resampling::R8brain(resampling) = settings.output().resampling() else {
            panic!("expected r8brain");
        };
        let low = R8brainQuality::Low.filter();
        assert_eq!(resampling.options.transition_band, low.transition_band);
        assert_eq!(resampling.options.attenuation, low.attenuation);
    }
}

#[gpui_kit::test]
fn another_value_makes_the_preset_custom_and_the_same_one_does_not(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.select_resampler(1);
    settings.select_r8brain_quality(1);
    let same = settings.snapshot().r8brain_filter().attenuation;
    settings.change_r8brain_filter(|filter| filter.attenuation = same);
    assert_eq!(settings.snapshot().r8brain_quality_ix(), 1);

    settings.change_r8brain_filter(|filter| filter.attenuation = 150.0);
    cx.run_until_parked();

    {
        let settings = settings.snapshot();
        assert_eq!(settings.r8brain_quality_ix(), 4);
        let medium = R8brainQuality::Medium.filter();
        assert_eq!(
            settings.r8brain_filter().transition_band,
            medium.transition_band
        );
    }
    let saved = store.r8brain_parameters().unwrap().quality;
    assert!(matches!(saved, R8brainQuality::Custom(_)));
    assert_eq!(saved.filter().attenuation, 150.0);
}

#[gpui_kit::test]
fn choosing_custom_keeps_the_sound_and_a_preset_replaces_its_values(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.select_resampler(1);
    settings.select_r8brain_quality(3);
    cx.run_until_parked();
    let told_before = told(&settings);

    settings.select_r8brain_quality(4);
    cx.run_until_parked();
    assert_eq!(told(&settings), told_before);
    {
        let settings = settings.snapshot();
        assert_eq!(settings.r8brain_quality_ix(), 4);
        assert_eq!(settings.r8brain_filter(), R8brainQuality::Max.filter());
    }

    settings.select_r8brain_quality(2);
    cx.run_until_parked();
    assert_eq!(
        settings.snapshot().r8brain_filter(),
        R8brainQuality::High.filter()
    );
    let text = std::fs::read_to_string(store.config_path()).unwrap();
    assert!(!text.contains("r8brain_attenuation"), "{text}");
}

#[gpui_kit::test]
fn a_preset_leaves_what_does_not_change_the_sound_alone(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.select_resampler(1);
    settings.change_r8brain(|parameters| {
        parameters.performance.extended_fft = true;
        parameters.performance.max_input_frames = 4096;
    });
    assert_eq!(settings.snapshot().r8brain_quality_ix(), 2);

    settings.select_r8brain_quality(0);
    {
        let settings = settings.snapshot();
        let performance = settings.r8brain_parameters().performance;
        assert!(performance.extended_fft);
        assert_eq!(performance.max_input_frames, 4096);
    }
}
