use gpui_kit::TestAppContext;

use super::*;

/// On GPUI's executor, which the test's `run_until_parked` drives. Without a cache directory
/// nothing on the disk or the network is touched.
fn without_a_cache_dir(store: Option<config::Store>, cx: &mut TestAppContext) -> Readings {
    let executor = cx.update(|cx| gpui_host::executor(cx));
    let net = net::Client::new("hiphile-test").unwrap();
    Readings::with(executor, net, store, None, false, None)
}

fn fetch(readings: &Readings) {
    readings.0.update(|state| readings.start_fetch(state));
}

fn failures(notices: &mut broadcast::Receiver<ReadingsFailed>) -> Vec<String> {
    std::iter::from_fn(|| notices.try_recv().ok())
        .map(|failed| failed.error().to_owned())
        .collect()
}

#[gpui_kit::test]
fn every_failure_says_what_went_wrong(cx: &mut TestAppContext) {
    let readings = without_a_cache_dir(None, cx);
    let mut notices = readings.notices();

    fetch(&readings);
    readings.retry();
    cx.run_until_parked();

    let failures = failures(&mut notices);
    assert_eq!(failures.len(), 2);
    assert!(failures[0].starts_with("Couldn't download the dictionaries: "));
    assert!(failures[0].ends_with('.'));
}

#[gpui_kit::test]
fn only_a_failure_is_told_as_one(cx: &mut TestAppContext) {
    let readings = without_a_cache_dir(None, cx);
    let mut notices = readings.notices();

    readings.0.update(|state| {
        readings.0.set_state(state, ReadingsState::Downloading(0.5));
        readings
            .0
            .set_state(state, ReadingsState::Ready("dir".into()));
        readings.0.set_state(state, ReadingsState::Off);
    });
    cx.run_until_parked();

    assert!(failures(&mut notices).is_empty());
}

#[gpui_kit::test]
fn each_change_is_published(cx: &mut TestAppContext) {
    let readings = without_a_cache_dir(None, cx);
    let mut changes = readings.subscribe();

    readings.set_on(true);
    assert!(changes.has_changed().unwrap());
    assert!(matches!(
        changes.borrow_and_update().state(),
        ReadingsState::Failed(_)
    ));

    readings.set_on(false);
    assert_eq!(*changes.borrow_and_update().state(), ReadingsState::Off);
}

#[gpui_kit::test]
fn the_choice_is_saved(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let readings = without_a_cache_dir(Some(store.clone()), cx);

    readings.set_on(true);
    cx.run_until_parked();
    assert!(store.romaji_search().unwrap());
    assert_eq!(readings.status().choice_error(), None);

    readings.set_on(false);
    cx.run_until_parked();
    assert!(!store.romaji_search().unwrap());
}

#[test]
fn the_saved_choice_is_read_and_is_on_when_it_cannot_be() {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    store.save_romaji_search(false).unwrap();
    let (kept, is_on, error) = read_saved_choice(Ok(store));
    assert!(kept.is_some());
    assert!(!is_on);
    assert_eq!(error, None);

    let (kept, is_on, error) = read_saved_choice(Err(config::Error::NoHomeDir));
    assert!(kept.is_none());
    assert!(is_on);
    assert!(
        error
            .unwrap()
            .starts_with("The search setting won't be saved: ")
    );
}

#[test]
fn only_ready_dictionaries_are_there_to_read_with() {
    let status = |state| ReadingsStatus {
        state,
        choice_error: None,
    };
    assert_eq!(
        status(ReadingsState::Ready("dir".into())).dictionaries(),
        Some(Path::new("dir"))
    );
    for state in [
        ReadingsState::Off,
        ReadingsState::Checking,
        ReadingsState::Downloading(0.5),
        ReadingsState::Failed("no".to_owned()),
    ] {
        assert_eq!(status(state).dictionaries(), None);
    }
}

#[test]
fn a_bundle_that_cannot_be_opened_goes() {
    let root = tempfile::tempdir().unwrap();
    assert_eq!(usable_bundle(root.path()), None);

    let gutted = root.path().join(BUNDLE.name);
    std::fs::create_dir(&gutted).unwrap();
    assert_eq!(usable_bundle(root.path()), None);
    assert!(!gutted.exists());
}

#[test]
fn progress_stays_a_fraction() {
    assert!((fraction_of(50, 100) - 0.5).abs() < f32::EPSILON);
    assert!((fraction_of(200, 100) - 1.).abs() < f32::EPSILON);
    assert!(fraction_of(1, 0).abs() < f32::EPSILON);
}
