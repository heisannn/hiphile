use config::{ResamplingQuality, SoxrInterpolation, SoxrParameters, SoxrPhase, SoxrQuality};
use gpui_kit::TestAppContext;

use crate::audio_options::Options;
use crate::audio_settings::tests::settings_saving_to;

const SOXR: usize = 2;

fn soxr_of(settings: &crate::AudioSettings) -> audio::SoxrSettings {
    let audio::Resampling::Soxr(resampling) = settings.output().resampling() else {
        panic!("expected soxr");
    };
    resampling
}

fn labels(options: &[(gpui_kit::SharedString, gpui_kit::SharedString)]) -> Vec<&str> {
    options.iter().map(|(_, label)| label.as_ref()).collect()
}

#[gpui_kit::test]
fn soxr_is_told_saved_and_reaches_the_output(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let (settings, told, _subscription) = settings_saving_to(&store, cx);

    settings.update(cx, |settings, cx| settings.select_resampler(SOXR, cx));
    cx.run_until_parked();

    assert_eq!(told.get(), 1);
    assert_eq!(
        store.resampling_quality().unwrap(),
        ResamplingQuality::Soxr(SoxrParameters::default())
    );
    settings.read_with(cx, |settings, _| {
        assert_eq!(settings.resampler_ix(), SOXR);
        assert!(settings.uses_soxr());
        assert!(!settings.uses_r8brain());
        assert_eq!(settings.soxr_quality_ix(), 3);
        assert_eq!(soxr_of(settings), audio::SoxrSettings::default());
    });
}

#[gpui_kit::test]
fn each_of_the_three_comes_back_at_the_quality_it_had(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let (settings, _told, _subscription) = settings_saving_to(&store, cx);
    settings.update(cx, |settings, cx| {
        settings.select_rubato_quality(1, cx);
        settings.select_resampler(1, cx);
        settings.select_r8brain_quality(0, cx);
        settings.select_resampler(SOXR, cx);
        settings.select_soxr_quality(1, cx);
        settings.select_resampler(0, cx);
    });
    cx.run_until_parked();
    assert_eq!(store.resampling_quality().unwrap(), ResamplingQuality::High);

    settings.update(cx, |settings, cx| settings.select_resampler(1, cx));
    settings.read_with(cx, |settings, _| {
        assert_eq!(settings.r8brain_quality_ix(), 0);
    });
    settings.update(cx, |settings, cx| settings.select_resampler(SOXR, cx));
    settings.read_with(cx, |settings, _| {
        assert_eq!(settings.soxr_quality_ix(), 1);
        assert_eq!(settings.rubato_quality_ix(), 1);
    });
}

#[gpui_kit::test]
fn a_changed_value_is_told_saved_and_reaches_the_output(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let (settings, told, _subscription) = settings_saving_to(&store, cx);
    settings.update(cx, |settings, cx| settings.select_resampler(SOXR, cx));

    settings.update(cx, |settings, cx| {
        settings.set_soxr_passband_end("0.95", cx);
        settings.set_soxr_precision("32", cx);
        settings.select_soxr_phase(1, cx);
    });
    cx.run_until_parked();

    assert_eq!(told.get(), 4);
    let saved = store.soxr_parameters().unwrap().quality.filter();
    assert_eq!(saved.passband_end, 0.95);
    assert_eq!(saved.precision, 32.0);
    assert_eq!(saved.phase, SoxrPhase::Intermediate);
    settings.read_with(cx, |settings, _| {
        let quality = soxr_of(settings).quality;
        assert_eq!(quality.passband_end, 0.95);
        assert_eq!(quality.precision, 32.0);
        assert_eq!(quality.phase_response, 25.0);
        assert_eq!(settings.soxr_quality_ix(), 4);
    });
}

#[gpui_kit::test]
fn the_values_come_back_after_another_resampler(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let (settings, _told, _subscription) = settings_saving_to(&store, cx);
    settings.update(cx, |settings, cx| {
        settings.select_resampler(SOXR, cx);
        settings.change_soxr_filter(|filter| filter.phase = SoxrPhase::Minimum, cx);
    });

    settings.update(cx, |settings, cx| settings.select_rubato_quality(2, cx));
    cx.run_until_parked();
    assert_eq!(store.resampling_quality().unwrap(), ResamplingQuality::Max);
    assert_eq!(
        store.soxr_parameters().unwrap().quality.filter().phase,
        SoxrPhase::Minimum
    );

    settings.update(cx, |settings, cx| settings.select_resampler(SOXR, cx));
    cx.run_until_parked();
    let ResamplingQuality::Soxr(parameters) = store.resampling_quality().unwrap() else {
        panic!("expected soxr");
    };
    assert_eq!(parameters.quality.filter().phase, SoxrPhase::Minimum);
}

#[gpui_kit::test]
fn a_change_that_comes_after_another_resampler_leaves_that_one(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let (settings, told, _subscription) = settings_saving_to(&store, cx);
    settings.update(cx, |settings, cx| settings.select_resampler(SOXR, cx));
    settings.update(cx, |settings, cx| settings.select_resampler(1, cx));
    cx.run_until_parked();
    assert_eq!(told.get(), 2);

    settings.update(cx, |settings, cx| settings.set_soxr_coef_table("1600", cx));
    cx.run_until_parked();

    assert_eq!(told.get(), 2);
    assert!(matches!(
        store.resampling_quality().unwrap(),
        ResamplingQuality::R8brain(_)
    ));
    assert_eq!(
        store.soxr_parameters().unwrap().performance.coef_table_kb,
        1600
    );
}

#[gpui_kit::test]
fn a_preset_is_told_saved_and_reaches_the_output(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let (settings, told, _subscription) = settings_saving_to(&store, cx);
    settings.update(cx, |settings, cx| settings.select_resampler(SOXR, cx));

    settings.update(cx, |settings, cx| settings.select_soxr_quality(2, cx));
    cx.run_until_parked();

    assert_eq!(told.get(), 2);
    assert_eq!(store.soxr_parameters().unwrap().quality, SoxrQuality::High);
    settings.read_with(cx, |settings, _| {
        assert_eq!(settings.soxr_quality_ix(), 2);
        let quality = soxr_of(settings).quality;
        assert_eq!(quality.precision, 20.0);
        assert_eq!(quality.engine().name(), "cr32");
    });
}

#[gpui_kit::test]
fn another_value_makes_the_preset_custom_and_the_same_one_does_not(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let (settings, _told, _subscription) = settings_saving_to(&store, cx);
    settings.update(cx, |settings, cx| {
        settings.select_resampler(SOXR, cx);
        settings.select_soxr_quality(1, cx);
        settings.set_soxr_precision("16", cx);
    });
    settings.read_with(cx, |settings, _| {
        assert_eq!(settings.soxr_quality_ix(), 1);
    });

    settings.update(cx, |settings, cx| settings.set_soxr_precision("24", cx));
    cx.run_until_parked();

    settings.read_with(cx, |settings, _| {
        assert_eq!(settings.soxr_quality_ix(), 4);
        assert_eq!(
            settings.soxr_filter().passband_end,
            SoxrQuality::Medium.filter().passband_end
        );
    });
    let saved = store.soxr_parameters().unwrap().quality;
    assert!(matches!(saved, SoxrQuality::Custom(_)));
    assert_eq!(saved.filter().precision, 24.0);
}

/// Low's rolloff is not High's in libsoxr, and changes nothing here.
#[gpui_kit::test]
fn choosing_custom_keeps_the_sound_and_a_preset_replaces_its_values(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let (settings, told, _subscription) = settings_saving_to(&store, cx);
    settings.update(cx, |settings, cx| {
        settings.select_resampler(SOXR, cx);
        settings.select_soxr_quality(0, cx);
    });
    cx.run_until_parked();
    let told_before = told.get();

    settings.update(cx, |settings, cx| settings.select_soxr_quality(4, cx));
    cx.run_until_parked();
    assert_eq!(told.get(), told_before);
    settings.read_with(cx, |settings, _| {
        assert_eq!(settings.soxr_quality_ix(), 4);
        assert_eq!(settings.soxr_filter(), SoxrQuality::Low.filter());
    });

    settings.update(cx, |settings, cx| settings.select_soxr_quality(3, cx));
    cx.run_until_parked();
    settings.read_with(cx, |settings, _| {
        assert_eq!(settings.soxr_filter(), SoxrQuality::VeryHigh.filter());
    });
    let text = std::fs::read_to_string(store.config_path()).unwrap();
    assert!(!text.contains("soxr_precision"), "{text}");
}

#[gpui_kit::test]
fn a_preset_leaves_what_does_not_change_the_sound_alone(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let (settings, _told, _subscription) = settings_saving_to(&store, cx);
    settings.update(cx, |settings, cx| {
        settings.select_resampler(SOXR, cx);
        settings.select_soxr_interpolation(2, cx);
        settings.set_soxr_min_dft("4096", cx);
        settings.set_soxr_large_dft("1048576", cx);
    });
    settings.read_with(cx, |settings, _| {
        assert_eq!(settings.soxr_quality_ix(), 3);
        let runtime = soxr_of(settings).runtime;
        assert_ne!(
            runtime.coef_interpolation,
            audio::SoxrSettings::default().runtime.coef_interpolation
        );
        assert_eq!(runtime.log2_min_dft_size, 12);
        assert_eq!(runtime.log2_large_dft_size, 20);
    });

    settings.update(cx, |settings, cx| settings.select_soxr_quality(0, cx));
    settings.read_with(cx, |settings, _| {
        let performance = settings.soxr_parameters().performance;
        assert_eq!(performance.coef_interpolation, SoxrInterpolation::Cubic);
        assert_eq!(performance.min_dft_size, 4096);
        assert_eq!(settings.soxr_interpolation_ix(), 2);
        assert_eq!(settings.soxr_min_dft_value().as_ref(), "4096");
        assert_eq!(settings.soxr_large_dft_value().as_ref(), "1048576");
    });
}

#[gpui_kit::test]
fn values_off_the_lists_stay_options_and_presets_read_apart(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let (settings, _told, _subscription) = settings_saving_to(&store, cx);
    settings.update(cx, |settings, cx| settings.select_resampler(SOXR, cx));
    settings.read_with(cx, |settings, _| {
        assert_eq!(
            labels(&settings.soxr_precision_options()),
            ["16-bit", "20-bit", "24-bit", "28-bit", "32-bit"]
        );
        assert_eq!(
            labels(&settings.soxr_passband_end_options()),
            ["0.90", "0.911", "0.95", "0.98", "0.99"]
        );
        assert_eq!(settings.soxr_precision_value().as_ref(), "28");
        assert_eq!(settings.soxr_coef_table_value().as_ref(), "400");
        assert_eq!(
            labels(&settings.soxr_coef_table_options()),
            ["100 kB", "200 kB", "400 kB", "800 kB", "1600 kB", "3200 kB"]
        );
    });

    let chosen = |options: Options, value: gpui_kit::SharedString| {
        let (_, label) = options.into_iter().find(|(of, _)| *of == value).unwrap();
        label.to_string()
    };
    let presets: Vec<String> = (0..4)
        .map(|ix| {
            settings.update(cx, |settings, cx| settings.select_soxr_quality(ix, cx));
            settings.read_with(cx, |settings, _| {
                chosen(
                    settings.soxr_passband_end_options(),
                    settings.soxr_passband_end_value(),
                )
            })
        })
        .collect();
    assert_eq!(presets, ["0.676", "0.916", "0.914", "0.911"]);

    settings.update(cx, |settings, cx| {
        settings.change_soxr_filter(|filter| filter.precision = 22.5, cx);
        settings.change_soxr(|parameters| parameters.performance.coef_table_kb = 1, cx);
    });
    settings.read_with(cx, |settings, _| {
        assert_eq!(
            labels(&settings.soxr_precision_options()),
            ["16-bit", "20-bit", "22.5-bit", "24-bit", "28-bit", "32-bit"]
        );
        assert_eq!(
            labels(&settings.soxr_coef_table_options()).first(),
            Some(&"1 kB")
        );
    });
}

#[gpui_kit::test]
fn a_phase_and_an_interpolation_are_chosen_by_their_places(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let (settings, _told, _subscription) = settings_saving_to(&store, cx);
    settings.update(cx, |settings, cx| settings.select_resampler(SOXR, cx));
    for (ix, phase) in [
        (2, SoxrPhase::Minimum),
        (1, SoxrPhase::Intermediate),
        (0, SoxrPhase::Linear),
    ] {
        settings.update(cx, |settings, cx| settings.select_soxr_phase(ix, cx));
        settings.read_with(cx, |settings, _| {
            assert_eq!(settings.soxr_phase_ix(), ix);
            assert_eq!(settings.soxr_filter().phase, phase);
        });
    }
    for (ix, interpolation) in [
        (1, SoxrInterpolation::Linear),
        (2, SoxrInterpolation::Cubic),
        (0, SoxrInterpolation::Auto),
    ] {
        settings.update(cx, |settings, cx| {
            settings.select_soxr_interpolation(ix, cx)
        });
        settings.read_with(cx, |settings, _| {
            assert_eq!(settings.soxr_interpolation_ix(), ix);
            assert_eq!(
                settings.soxr_parameters().performance.coef_interpolation,
                interpolation
            );
        });
    }
}
