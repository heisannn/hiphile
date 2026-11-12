use config::{R8brainParameters, R8brainPhase, R8brainQuality, ResamplingQuality};
use executor::testing::TestExecutor;

use super::{R8brainNumber, R8brainSetting, SETTLE};
use crate::settings::audio::tests::{settings_saving_to, told};
use crate::settings::{AudioSettings, AudioSnapshot};

/// Types `text` into the number's field and presses Enter.
fn enter(settings: &AudioSettings, number: R8brainNumber, text: &str) {
    settings.type_r8brain(number, text);
    settings.commit_r8brain(number);
}

/// On r8brain, at its default quality.
fn r8brain_saving_to(store: &config::Store, cx: &TestExecutor) -> AudioSettings {
    let settings = settings_saving_to(store, cx);
    settings.select_resampler(1);
    settings
}

fn settle(cx: &TestExecutor) {
    cx.advance_clock(SETTLE);
    cx.run_until_parked();
}

#[test]
fn a_changed_parameter_is_told_saved_and_reaches_the_output() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.select_resampler(1);

    enter(&settings, R8brainNumber::TransitionBand, "3.5");
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

#[test]
fn the_parameters_come_back_after_another_quality() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.select_resampler(1);
    enter(&settings, R8brainNumber::Attenuation, "100");
    settings.select_r8brain_phase(1);

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

#[test]
fn a_change_that_comes_after_another_quality_leaves_that_quality() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.select_resampler(1);
    settings.select_rubato_quality(2);
    cx.run_until_parked();
    assert_eq!(told(&settings), 2);

    enter(&settings, R8brainNumber::Attenuation, "100");
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

#[test]
fn a_bit_depth_sets_the_attenuation_and_other_values_read_other() {
    let cx = &TestExecutor::new();
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

    enter(&settings, R8brainNumber::Attenuation, "140");
    {
        let settings = settings.snapshot();
        assert_eq!(settings.r8brain_bit_depth_value(), "Other");
        assert!(labels(&settings).contains(&"Other".to_owned()));
    }
}

#[test]
fn a_phase_is_chosen_by_its_place() {
    let cx = &TestExecutor::new();
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

#[test]
fn a_preset_is_told_saved_and_reaches_the_output() {
    let cx = &TestExecutor::new();
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

#[test]
fn another_value_makes_the_preset_custom_and_the_same_one_does_not() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.select_resampler(1);
    settings.select_r8brain_quality(1);
    let same = settings.snapshot().r8brain_filter().attenuation;
    enter(&settings, R8brainNumber::Attenuation, &same.to_string());
    assert_eq!(settings.snapshot().r8brain_quality_ix(), 1);

    enter(&settings, R8brainNumber::Attenuation, "150");
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

#[test]
fn choosing_custom_keeps_the_sound_and_a_preset_replaces_its_values() {
    let cx = &TestExecutor::new();
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

#[test]
fn a_preset_leaves_what_does_not_change_the_sound_alone() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.select_resampler(1);
    settings.set_r8brain_extended_fft(true);
    enter(&settings, R8brainNumber::MaxInputFrames, "4096");
    assert_eq!(settings.snapshot().r8brain_quality_ix(), 2);

    settings.select_r8brain_quality(0);
    {
        let settings = settings.snapshot();
        let performance = settings.r8brain_parameters().performance;
        assert!(performance.extended_fft);
        assert_eq!(performance.max_input_frames, 4096);
    }
}

#[test]
fn a_typed_number_counts_once_it_has_settled() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = r8brain_saving_to(&store, cx);
    settings.type_r8brain(R8brainNumber::TransitionBand, "3.5");
    cx.run_until_parked();
    assert_eq!(settings.snapshot().r8brain_filter().transition_band, 2.0);

    settle(cx);

    assert_eq!(settings.snapshot().r8brain_filter().transition_band, 3.5);
    let saved = store.r8brain_parameters().unwrap().quality.filter();
    assert_eq!(saved.transition_band, 3.5);
}

#[test]
fn only_the_last_of_several_typed_numbers_counts() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = r8brain_saving_to(&store, cx);
    for text in ["1", "10", "100"] {
        settings.type_r8brain(R8brainNumber::Attenuation, text);
    }
    settle(cx);
    assert_eq!(settings.snapshot().r8brain_filter().attenuation, 100.0);
}

#[test]
fn enter_counts_at_once_held_to_the_range() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = r8brain_saving_to(&store, cx);
    enter(&settings, R8brainNumber::Attenuation, "999");
    let snapshot = settings.snapshot();
    assert_eq!(snapshot.r8brain_filter().attenuation, 218.0);
    assert_eq!(snapshot.r8brain_text(R8brainNumber::Attenuation), "218");
}

#[test]
fn a_typed_number_beyond_the_range_waits_for_enter() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = r8brain_saving_to(&store, cx);
    settings.type_r8brain(R8brainNumber::Attenuation, "1");
    settle(cx);
    let snapshot = settings.snapshot();
    assert_eq!(snapshot.r8brain_parameters().quality, R8brainQuality::High);
    assert_eq!(snapshot.r8brain_text(R8brainNumber::Attenuation), "1");

    settings.commit_r8brain(R8brainNumber::Attenuation);

    let snapshot = settings.snapshot();
    assert_eq!(snapshot.r8brain_filter().attenuation, 49.0);
    assert_eq!(snapshot.r8brain_text(R8brainNumber::Attenuation), "49");
}

#[test]
fn text_that_is_no_number_gives_way_to_the_number() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = r8brain_saving_to(&store, cx);
    enter(&settings, R8brainNumber::TransitionBand, "");
    let snapshot = settings.snapshot();
    assert_eq!(snapshot.r8brain_filter().transition_band, 2.0);
    assert_eq!(snapshot.r8brain_text(R8brainNumber::TransitionBand), "2");
}

#[test]
fn no_filter_positions_let_the_resampler_choose_them() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = r8brain_saving_to(&store, cx);
    let positions = R8brainNumber::FilterFractions;
    assert_eq!(settings.snapshot().r8brain_text(positions), "");
    assert_eq!(positions.placeholder(), Some("Auto"));

    enter(&settings, positions, "512");
    assert_eq!(
        settings.snapshot().r8brain_filter().filter_fractions,
        Some(512)
    );

    enter(&settings, positions, "");
    let snapshot = settings.snapshot();
    assert_eq!(snapshot.r8brain_filter().filter_fractions, None);
    assert_eq!(snapshot.r8brain_text(positions), "");
}

#[test]
fn whole_numbers_stay_whole() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = r8brain_saving_to(&store, cx);
    enter(&settings, R8brainNumber::FilterCacheSize, "3.6");
    let performance = settings.snapshot().r8brain_parameters().performance;
    assert_eq!(performance.filter_cache_size, 4);
}

#[test]
fn a_preset_replaces_a_typed_number() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = r8brain_saving_to(&store, cx);
    settings.type_r8brain(R8brainNumber::Attenuation, "150");
    settings.type_r8brain(R8brainNumber::MaxInputFrames, "4096");

    settings.select_r8brain_quality(0);
    let snapshot = settings.snapshot();
    assert_eq!(snapshot.r8brain_text(R8brainNumber::Attenuation), "136.45");
    assert_eq!(snapshot.r8brain_text(R8brainNumber::MaxInputFrames), "4096");

    settle(cx);
    settings.commit_r8brain(R8brainNumber::Attenuation);
    let snapshot = settings.snapshot();
    assert_eq!(snapshot.r8brain_parameters().quality, R8brainQuality::Low);
    assert_eq!(
        snapshot.r8brain_parameters().performance.max_input_frames,
        4096
    );
}

#[test]
fn a_typed_number_keeps_its_form_until_enter() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = r8brain_saving_to(&store, cx);
    let band = R8brainNumber::TransitionBand;
    settings.type_r8brain(band, "3.50");
    settle(cx);
    let snapshot = settings.snapshot();
    assert_eq!(snapshot.r8brain_filter().transition_band, 3.5);
    assert_eq!(snapshot.r8brain_text(band), "3.50");

    settings.commit_r8brain(band);
    assert_eq!(settings.snapshot().r8brain_text(band), "3.5");
}

#[test]
fn another_number_counting_leaves_a_typed_one_alone() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = r8brain_saving_to(&store, cx);
    settings.type_r8brain(R8brainNumber::Attenuation, "150");
    enter(&settings, R8brainNumber::MaxInputFrames, "4096");
    enter(&settings, R8brainNumber::TransitionBand, "3");
    assert_eq!(
        settings.snapshot().r8brain_text(R8brainNumber::Attenuation),
        "150"
    );

    settle(cx);

    assert_eq!(settings.snapshot().r8brain_filter().attenuation, 150.0);
}

#[test]
fn what_leaves_the_sound_alone_leaves_the_preset_too() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = r8brain_saving_to(&store, cx);
    enter(&settings, R8brainNumber::MaxInputFrames, "4096");
    settings.set_r8brain_extended_fft(true);
    assert_eq!(
        settings.snapshot().r8brain_parameters().quality,
        R8brainQuality::High
    );

    settings.set_r8brain_whole_stepping(false);
    assert_eq!(settings.snapshot().r8brain_quality_ix(), 4);
}

#[test]
fn a_number_goes_back_to_its_default() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = r8brain_saving_to(&store, cx);
    let cache = R8brainSetting::Number(R8brainNumber::FilterBankCacheSize);
    assert!(!settings.snapshot().is_r8brain_resettable(cache));
    enter(&settings, R8brainNumber::FilterBankCacheSize, "3");
    assert!(settings.snapshot().is_r8brain_resettable(cache));

    settings.type_r8brain(R8brainNumber::FilterBankCacheSize, "5");
    settings.reset_r8brain(cache);
    settle(cx);

    let snapshot = settings.snapshot();
    assert_eq!(
        snapshot
            .r8brain_parameters()
            .performance
            .filter_bank_cache_size,
        12
    );
    assert_eq!(
        snapshot.r8brain_text(R8brainNumber::FilterBankCacheSize),
        "12"
    );
}

#[test]
fn numbers_that_differ_between_presets_go_back_with_the_preset() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = r8brain_saving_to(&store, cx);
    settings.select_r8brain_quality(0);
    for number in [R8brainNumber::Attenuation, R8brainNumber::TransitionBand] {
        let setting = R8brainSetting::Number(number);
        assert!(!settings.snapshot().is_r8brain_resettable(setting));
        enter(&settings, number, "10");
        assert!(!settings.snapshot().is_r8brain_resettable(setting));
        settings.reset_r8brain(setting);
        assert_eq!(settings.snapshot().r8brain_quality_ix(), 4);
    }

    settings.reset_r8brain(R8brainSetting::Quality);
    assert_eq!(
        settings.snapshot().r8brain_parameters().quality,
        R8brainQuality::default()
    );
}

#[test]
fn the_switches_and_the_phase_go_back_to_their_defaults() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = r8brain_saving_to(&store, cx);
    settings.select_r8brain_phase(1);
    settings.set_r8brain_whole_stepping(false);
    settings.set_r8brain_extended_fft(true);
    let changed = [
        R8brainSetting::Phase,
        R8brainSetting::WholeStepping,
        R8brainSetting::ExtendedFft,
    ];
    assert!(
        changed
            .iter()
            .all(|setting| settings.snapshot().is_r8brain_resettable(*setting))
    );

    for setting in changed {
        settings.reset_r8brain(setting);
    }

    let snapshot = settings.snapshot();
    assert!(
        !changed
            .iter()
            .any(|setting| snapshot.is_r8brain_resettable(*setting))
    );
    assert_eq!(
        snapshot.r8brain_filter(),
        R8brainQuality::default().filter()
    );
    assert_eq!(
        snapshot.r8brain_parameters().performance,
        R8brainParameters::default().performance
    );
}

#[test]
fn every_number_of_the_defaults_is_in_its_range() {
    let parameters = R8brainParameters::default();
    for number in R8brainNumber::ALL {
        if let Some(value) = number.value(&parameters) {
            assert_eq!(number.range().parse(&value.to_string()), Some(value));
        }
    }
}
