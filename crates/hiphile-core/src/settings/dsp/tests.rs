use gpui_kit::TestAppContext;

use super::*;

/// On GPUI's executor, which the test's `run_until_parked` drives.
fn settings_saving_to(store: &config::Store, cx: &mut TestAppContext) -> DspSettings {
    let executor = cx.update(|cx| gpui_host::executor(cx));
    DspSettings::with(
        executor,
        Some(store.clone()),
        Crossfeed::default(),
        0.0,
        Vec::new(),
    )
}

fn crossfeed(is_on: bool, preset: CrossfeedPreset) -> Crossfeed {
    let mut crossfeed = Crossfeed::default();
    crossfeed.is_on = is_on;
    crossfeed.preset = preset;
    crossfeed
}

fn written(store: &config::Store) -> String {
    std::fs::read_to_string(store.config_path()).unwrap_or_default()
}

#[gpui_kit::test]
fn turning_crossfeed_on_is_told_saved_and_plays_bs2b(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    let told = settings.subscribe();

    settings.set_crossfeed(false);
    cx.run_until_parked();
    assert!(!told.has_changed().unwrap());
    assert!(!store.config_path().exists());

    settings.set_crossfeed(true);
    cx.run_until_parked();
    assert!(told.has_changed().unwrap());
    assert!(store.crossfeed().unwrap().is_on);
    let snapshot = settings.snapshot();
    assert!(snapshot.is_crossfeed_on());
    assert_eq!(snapshot.crossfeed_preset_ix(), 0);
    assert_eq!(
        snapshot.settings().crossfeed(),
        audio::Crossfeed::new(700, 4.5)
    );
}

#[gpui_kit::test]
fn changing_a_value_makes_the_preset_custom_with_the_values_in_use(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.set_crossfeed(true);
    settings.select_crossfeed_preset(2);

    settings.set_crossfeed_cutoff("650");
    assert_eq!(settings.snapshot().crossfeed_preset_ix(), 2);
    settings.set_crossfeed_cutoff("700");
    cx.run_until_parked();

    let snapshot = settings.snapshot();
    assert_eq!(snapshot.crossfeed_preset_ix(), CUSTOM_IX);
    assert_eq!(snapshot.cutoff_value(), "700");
    assert_eq!(snapshot.separation_value(), "9.5");
    let filter = store.crossfeed().unwrap().preset.filter();
    assert_eq!((filter.cutoff_hz, filter.separation_db), (700, 9.5));
}

#[gpui_kit::test]
fn choosing_custom_changes_nothing_that_plays(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.set_crossfeed(true);
    settings.select_crossfeed_preset(1);
    let chu_moy = settings.snapshot().settings();

    settings.select_crossfeed_preset(CUSTOM_IX);
    assert_eq!(settings.snapshot().crossfeed_preset_ix(), CUSTOM_IX);
    assert_eq!(settings.snapshot().settings(), chu_moy);
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_preset_stays_while_crossfeed_is_off(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    settings.set_crossfeed(true);
    settings.select_crossfeed_preset(2);

    settings.set_crossfeed(false);
    let snapshot = settings.snapshot();
    assert_eq!(snapshot.settings().crossfeed(), None);
    assert_eq!(snapshot.crossfeed_preset_ix(), 2);

    settings.set_crossfeed(true);
    cx.run_until_parked();
    assert_eq!(
        settings.snapshot().settings().crossfeed(),
        audio::Crossfeed::new(650, 9.5)
    );
    assert_eq!(store.crossfeed().unwrap().preset, CrossfeedPreset::JanMeier);
}

#[gpui_kit::test]
fn a_balance_goes_by_half_a_db_up_to_12_and_names_the_lowered_side(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);
    assert_eq!(settings.snapshot().balance_text(), "Center");

    settings.set_balance(-1.3);
    assert_eq!(settings.snapshot().balance_db(), -1.5);
    assert_eq!(settings.snapshot().balance_text(), "Right -1.5 dB");
    settings.set_balance(20.0);
    assert_eq!(settings.snapshot().balance_text(), "Left -12.0 dB");
    assert_eq!(
        settings.snapshot().settings().balance(),
        audio::Balance::from_db(12.0)
    );
    cx.run_until_parked();
    assert_eq!(store.balance_db().unwrap(), 12.0);

    settings.set_balance(f64::NAN);
    cx.run_until_parked();
    assert_eq!(settings.snapshot().balance_text(), "Center");
    assert!(!written(&store).contains("balance_db"));
}

#[gpui_kit::test]
fn only_what_changed_is_written(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let executor = cx.update(|cx| gpui_host::executor(cx));
    let from_environment = crossfeed(true, CrossfeedPreset::JanMeier);
    let settings = DspSettings::with(
        executor,
        Some(store.clone()),
        from_environment,
        0.0,
        Vec::new(),
    );

    settings.set_balance(-2.0);
    cx.run_until_parked();
    assert_eq!(written(&store).trim(), "balance_db = -2.0");

    settings.set_crossfeed_separation("12");
    cx.run_until_parked();
    let text = written(&store);
    assert!(text.contains("crossfeed_preset = \"custom\""), "{text}");
    assert!(text.contains("crossfeed_cutoff_hz = 650"), "{text}");
    assert!(text.contains("crossfeed_separation_db = 12.0"), "{text}");
    assert!(!text.contains("crossfeed = true"), "{text}");
}

#[gpui_kit::test]
fn what_waits_is_written_before_quitting(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let settings = settings_saving_to(&store, cx);

    settings.set_crossfeed(true);
    settings.select_crossfeed_preset(1);
    settings.set_balance(3.0);
    settings.save_before_quitting();

    let crossfeed = store.crossfeed().unwrap();
    assert!(crossfeed.is_on);
    assert_eq!(crossfeed.preset, CrossfeedPreset::ChuMoy);
    assert_eq!(store.balance_db().unwrap(), 3.0);
    cx.run_until_parked();
}

#[test]
fn a_value_that_cannot_be_read_is_named_and_the_rest_still_applies() {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    std::fs::write(
        store.config_path(),
        "crossfeed_preset = \"strong\"\nbalance_db = -3.0\n",
    )
    .unwrap();
    let (_, crossfeed, balance_db, errors) = read_saved(Ok(store));
    assert_eq!(crossfeed, Crossfeed::default());
    assert_eq!(balance_db, -3.0);
    let [error] = errors.as_slice() else {
        panic!("{errors:?}");
    };
    assert!(
        error.starts_with("Couldn't read the DSP settings:"),
        "{error}"
    );
    assert!(error.contains("crossfeed_preset"), "{error}");
}

#[test]
fn each_value_that_cannot_be_read_is_named() {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    std::fs::write(
        store.config_path(),
        "crossfeed_preset = \"strong\"\nbalance_db = 30.0\n",
    )
    .unwrap();
    let (_, _, balance_db, errors) = read_saved(Ok(store));
    assert_eq!(balance_db, 0.0);
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert!(errors[0].contains("crossfeed_preset"), "{}", errors[0]);
    assert!(errors[1].contains("balance_db"), "{}", errors[1]);
}

#[test]
fn the_options_hold_every_preset_and_a_value_from_the_file() {
    let snapshot = |preset| DspSnapshot {
        crossfeed: crossfeed(true, preset),
        balance_db: 0.0,
        read_errors: Vec::new(),
        save_error: None,
    };
    for preset in [
        CrossfeedPreset::Bs2b,
        CrossfeedPreset::ChuMoy,
        CrossfeedPreset::JanMeier,
    ] {
        let snapshot = snapshot(preset);
        let has = |options: Vec<Choice>, value: String| {
            options.iter().any(|option| option.value() == value)
        };
        assert!(has(snapshot.cutoff_options(), snapshot.cutoff_value()));
        assert!(has(
            snapshot.separation_options(),
            snapshot.separation_value()
        ));
        assert_eq!(snapshot.cutoff_options().len(), 35);
        assert_eq!(snapshot.separation_options().len(), 29);
    }
    let mut filter = CrossfeedPreset::Bs2b.filter();
    filter.cutoff_hz = 777;
    let custom = snapshot(CrossfeedPreset::Custom(filter));
    let options = custom.cutoff_options();
    assert_eq!(options.len(), 36);
    assert_eq!(options.last().unwrap().label(), "777 Hz");
    assert_eq!(custom.separation_options()[7].label(), "4.5 dB");
}
