use gpui_kit::TestAppContext;
use tempfile::TempDir;

use super::*;

fn hiphile(store: Result<config::Store, config::Error>, cx: &mut TestAppContext) -> Hiphile {
    let executor = cx.update(|cx| gpui_host::executor(cx));
    let net = net::Client::new("hiphile-test").unwrap();
    Hiphile::new(executor, net, store, None, Caches::None)
}

fn store_with(config: &str) -> (TempDir, config::Store) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.toml"), config).unwrap();
    let store = config::Store::at(dir.path());
    (dir, store)
}

#[gpui_kit::test]
fn every_setting_reads_the_one_store(cx: &mut TestAppContext) {
    let (_dir, store) = store_with(
        "skip_instrumentals = true\nlyrics_highlight = \"line\"\nromaji_search = false\n",
    );

    let hiphile = hiphile(Ok(store), cx);

    let settings = hiphile.settings();
    assert!(settings.queue().snapshot().is_skip_instrumentals_on());
    assert_eq!(
        settings.lyrics().snapshot().highlight(),
        config::LyricsHighlight::ByLine
    );
    assert!(!hiphile.readings().status().is_on());
    cx.run_until_parked();
}

#[gpui_kit::test]
fn without_a_store_each_setting_says_why_it_will_not_be_saved(cx: &mut TestAppContext) {
    let hiphile = hiphile(Err(config::Error::NoHomeDir), cx);

    let settings = hiphile.settings();
    let errors = [
        settings.queue().snapshot().error().map(str::to_owned),
        settings.lyrics().snapshot().error().map(str::to_owned),
        settings.appearance().snapshot().error().map(str::to_owned),
        settings.song_cache().snapshot().error().map(str::to_owned),
    ];
    for error in errors {
        let error = error.expect("no error");
        assert!(error.contains("won't be saved"), "{error}");
        assert!(error.contains("home directory"), "{error}");
    }
    cx.run_until_parked();
}

#[gpui_kit::test]
fn with_nowhere_to_keep_them_there_are_no_covers_or_dictionaries(cx: &mut TestAppContext) {
    let (_dir, store) = store_with("");

    let hiphile = hiphile(Ok(store), cx);

    assert!(hiphile.covers().is_none());
    cx.run_until_parked();
    assert!(matches!(
        hiphile.readings().status().state(),
        session::ReadingsState::Failed(_)
    ));
}

#[gpui_kit::test]
fn signing_out_closes_the_workspace(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let hiphile = hiphile(Err(config::Error::NoHomeDir), cx);
    let changes = hiphile.subscribe();
    let profile = config::Profile::new(crate::testing::NOWHERE, "alice");
    hiphile.open_workspace(
        crate::test_support::session(crate::testing::NOWHERE),
        profile.clone(),
    );
    assert_eq!(hiphile.snapshot().profile(), Some(&profile));
    assert_eq!(hiphile.snapshot().workspace(), hiphile.workspace().as_ref());

    hiphile.sign_out();

    assert_eq!(hiphile.snapshot(), HiphileSnapshot::default());
    assert!(hiphile.workspace().is_none());
    assert!(changes.has_changed().unwrap());
    cx.run_until_parked();
}

#[gpui_kit::test]
fn what_was_typed_is_checked_before_signing_in(cx: &mut TestAppContext) {
    let hiphile = hiphile(Err(config::Error::NoHomeDir), cx);

    let invalid = hiphile.sign_in("", "alice", "secret").err().unwrap();

    assert_eq!(invalid.field(), session::SignInField::Server);
    assert!(hiphile.workspace().is_none());
    cx.run_until_parked();
}
