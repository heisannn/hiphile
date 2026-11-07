use config::{R8brainPhase, R8brainQuality, ResamplingQuality};
use gpui_kit::TestAppContext;

use crate::audio_settings::tests::settings_saving_to;

#[gpui_kit::test]
fn a_changed_parameter_is_told_saved_and_reaches_the_output(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let (settings, told, _subscription) = settings_saving_to(&store, cx);
    settings.update(cx, |settings, cx| settings.select_resampler(1, cx));

    settings.update(cx, |settings, cx| {
        settings.change_r8brain_filter(|filter| filter.transition_band = 3.5, cx);
    });
    cx.run_until_parked();

    assert_eq!(told.get(), 2);
    let saved = store.r8brain_parameters().unwrap();
    assert_eq!(saved.quality.filter().transition_band, 3.5);
    settings.read_with(cx, |settings, _| {
        let mut expected = audio::R8brainSettings::default();
        expected.options.transition_band = 3.5;
        assert_eq!(
            settings.output().resampling(),
            audio::Resampling::R8brain(expected)
        );
    });
}

#[gpui_kit::test]
fn the_parameters_come_back_after_another_quality(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let (settings, _told, _subscription) = settings_saving_to(&store, cx);
    settings.update(cx, |settings, cx| {
        settings.select_resampler(1, cx);
        settings.change_r8brain_filter(
            |filter| {
                filter.attenuation = 100.0;
                filter.phase = R8brainPhase::Minimum;
            },
            cx,
        );
    });

    settings.update(cx, |settings, cx| settings.select_rubato_quality(2, cx));
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

    settings.update(cx, |settings, cx| settings.select_resampler(1, cx));
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
    let (settings, told, _subscription) = settings_saving_to(&store, cx);
    settings.update(cx, |settings, cx| settings.select_resampler(1, cx));
    settings.update(cx, |settings, cx| settings.select_rubato_quality(2, cx));
    cx.run_until_parked();
    assert_eq!(told.get(), 2);

    settings.update(cx, |settings, cx| {
        settings.change_r8brain_filter(|filter| filter.attenuation = 100.0, cx);
    });
    cx.run_until_parked();

    assert_eq!(told.get(), 2);
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
    let (settings, _told, _subscription) = settings_saving_to(&store, cx);
    settings.update(cx, |settings, cx| settings.select_resampler(1, cx));
    let labels = |settings: &crate::AudioSettings| -> Vec<String> {
        settings
            .r8brain_bit_depth_options()
            .into_iter()
            .map(|(value, _)| value.to_string())
            .collect()
    };

    settings.read_with(cx, |settings, _| {
        assert_eq!(settings.r8brain_bit_depth_value().as_ref(), "24-bit");
        assert!(!labels(settings).contains(&"Other".to_owned()));
    });

    settings.update(cx, |settings, cx| {
        settings.set_r8brain_bit_depth("16-bit", cx)
    });
    settings.read_with(cx, |settings, _| {
        assert_eq!(settings.r8brain_filter().attenuation, 136.45);
    });

    settings.update(cx, |settings, cx| {
        settings.change_r8brain_filter(|filter| filter.attenuation = 140.0, cx);
    });
    settings.read_with(cx, |settings, _| {
        assert_eq!(settings.r8brain_bit_depth_value().as_ref(), "Other");
        assert!(labels(settings).contains(&"Other".to_owned()));
    });
}

#[gpui_kit::test]
fn a_phase_is_chosen_by_its_place(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let (settings, _told, _subscription) = settings_saving_to(&store, cx);
    settings.update(cx, |settings, cx| settings.select_resampler(1, cx));
    settings.read_with(cx, |settings, _| assert_eq!(settings.r8brain_phase_ix(), 0));

    settings.update(cx, |settings, cx| settings.select_r8brain_phase(1, cx));
    settings.read_with(cx, |settings, _| {
        assert_eq!(settings.r8brain_phase_ix(), 1);
        assert_eq!(settings.r8brain_filter().phase, R8brainPhase::Minimum);
    });
}

#[gpui_kit::test]
fn a_preset_is_told_saved_and_reaches_the_output(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let (settings, told, _subscription) = settings_saving_to(&store, cx);
    settings.update(cx, |settings, cx| settings.select_resampler(1, cx));
    settings.read_with(cx, |settings, _| {
        assert_eq!(settings.r8brain_quality_ix(), 2);
    });

    settings.update(cx, |settings, cx| settings.select_r8brain_quality(0, cx));
    cx.run_until_parked();

    assert_eq!(told.get(), 2);
    assert_eq!(
        store.r8brain_parameters().unwrap().quality,
        R8brainQuality::Low
    );
    settings.read_with(cx, |settings, _| {
        assert_eq!(settings.r8brain_quality_ix(), 0);
        let audio::Resampling::R8brain(resampling) = settings.output().resampling() else {
            panic!("expected r8brain");
        };
        let low = R8brainQuality::Low.filter();
        assert_eq!(resampling.options.transition_band, low.transition_band);
        assert_eq!(resampling.options.attenuation, low.attenuation);
    });
}

#[gpui_kit::test]
fn another_value_makes_the_preset_custom_and_the_same_one_does_not(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let (settings, _told, _subscription) = settings_saving_to(&store, cx);
    settings.update(cx, |settings, cx| {
        settings.select_resampler(1, cx);
        settings.select_r8brain_quality(1, cx);
        let same = settings.r8brain_filter().attenuation;
        settings.change_r8brain_filter(|filter| filter.attenuation = same, cx);
    });
    settings.read_with(cx, |settings, _| {
        assert_eq!(settings.r8brain_quality_ix(), 1);
    });

    settings.update(cx, |settings, cx| {
        settings.change_r8brain_filter(|filter| filter.attenuation = 150.0, cx);
    });
    cx.run_until_parked();

    settings.read_with(cx, |settings, _| {
        assert_eq!(settings.r8brain_quality_ix(), 4);
        let medium = R8brainQuality::Medium.filter();
        assert_eq!(
            settings.r8brain_filter().transition_band,
            medium.transition_band
        );
    });
    let saved = store.r8brain_parameters().unwrap().quality;
    assert!(matches!(saved, R8brainQuality::Custom(_)));
    assert_eq!(saved.filter().attenuation, 150.0);
}

#[gpui_kit::test]
fn choosing_custom_keeps_the_sound_and_a_preset_replaces_its_values(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let (settings, told, _subscription) = settings_saving_to(&store, cx);
    settings.update(cx, |settings, cx| {
        settings.select_resampler(1, cx);
        settings.select_r8brain_quality(3, cx);
    });
    cx.run_until_parked();
    let told_before = told.get();

    settings.update(cx, |settings, cx| settings.select_r8brain_quality(4, cx));
    cx.run_until_parked();
    assert_eq!(told.get(), told_before);
    settings.read_with(cx, |settings, _| {
        assert_eq!(settings.r8brain_quality_ix(), 4);
        assert_eq!(settings.r8brain_filter(), R8brainQuality::Max.filter());
    });

    settings.update(cx, |settings, cx| settings.select_r8brain_quality(2, cx));
    cx.run_until_parked();
    settings.read_with(cx, |settings, _| {
        assert_eq!(settings.r8brain_filter(), R8brainQuality::High.filter());
    });
    let text = std::fs::read_to_string(store.config_path()).unwrap();
    assert!(!text.contains("r8brain_attenuation"), "{text}");
}

#[gpui_kit::test]
fn a_preset_leaves_what_does_not_change_the_sound_alone(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let (settings, _told, _subscription) = settings_saving_to(&store, cx);
    settings.update(cx, |settings, cx| {
        settings.select_resampler(1, cx);
        settings.change_r8brain(
            |parameters| {
                parameters.performance.extended_fft = true;
                parameters.performance.max_input_frames = 4096;
            },
            cx,
        );
    });
    settings.read_with(cx, |settings, _| {
        assert_eq!(settings.r8brain_quality_ix(), 2);
    });

    settings.update(cx, |settings, cx| settings.select_r8brain_quality(0, cx));
    settings.read_with(cx, |settings, _| {
        let performance = settings.r8brain_parameters().performance;
        assert!(performance.extended_fft);
        assert_eq!(performance.max_input_frames, 4096);
    });
}
