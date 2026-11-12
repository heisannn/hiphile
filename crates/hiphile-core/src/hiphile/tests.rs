use executor::testing::TestExecutor;
use tempfile::TempDir;

use super::*;
use crate::testing::{NOWHERE, server_giving_image, server_never_answering, wait_for};

fn hiphile(store: Result<config::Store, config::Error>, cx: &TestExecutor) -> Hiphile {
    hiphile_keeping(store, Caches::None, cx)
}

fn hiphile_keeping(
    store: Result<config::Store, config::Error>,
    caches: Caches,
    cx: &TestExecutor,
) -> Hiphile {
    let executor = cx.executor();
    let net = net::Client::new("hiphile-test").unwrap();
    Hiphile::new(executor, net, store, None, caches)
}

fn store_with(config: &str) -> (TempDir, config::Store) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.toml"), config).unwrap();
    let store = config::Store::at(dir.path());
    (dir, store)
}

#[test]
fn every_setting_reads_the_one_store() {
    let cx = &TestExecutor::new();
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

#[test]
fn without_a_store_each_setting_says_why_it_will_not_be_saved() {
    let cx = &TestExecutor::new();
    let hiphile = hiphile(Err(config::Error::NoHomeDir), cx);

    let settings = hiphile.settings();
    let errors = [
        settings.queue().snapshot().error().map(str::to_owned),
        settings.lyrics().snapshot().error().map(str::to_owned),
        settings
            .appearance()
            .snapshot()
            .errors()
            .next()
            .map(str::to_owned),
        settings.song_cache().snapshot().error().map(str::to_owned),
    ];
    for error in errors {
        let error = error.expect("no error");
        assert!(error.contains("won't be saved"), "{error}");
        assert!(error.contains("home directory"), "{error}");
    }
    cx.run_until_parked();
}

#[test]
fn with_nowhere_to_keep_them_there_are_no_covers_or_dictionaries() {
    let cx = &TestExecutor::new();
    let (_dir, store) = store_with("");

    let hiphile = hiphile(Ok(store), cx);

    assert!(hiphile.covers().is_none());
    cx.run_until_parked();
    assert!(matches!(
        hiphile.readings().status().state(),
        session::ReadingsState::Failed(_)
    ));
}

#[test]
fn a_folder_the_host_gives_keeps_the_covers_and_the_dictionaries() {
    let cx = &TestExecutor::new();
    let (_dir, store) = store_with("romaji_search = false\n");
    let cache = tempfile::tempdir().unwrap();
    let dictionaries = cache.path().join("readings");
    std::fs::create_dir(&dictionaries).unwrap();

    let hiphile = hiphile_keeping(Ok(store), Caches::At(cache.path().to_owned()), cx);
    let url = url::Url::parse("http://127.0.0.1:9/rest/getCoverArt?id=al-1&size=80").unwrap();
    let cover = subsonic::CoverArtRequest::parse(&url).unwrap();
    hiphile.covers().unwrap().put(&cover, b"image").unwrap();
    hiphile.readings();
    cx.run_until_parked();

    assert!(cache.path().join("servers").is_dir());
    // Turned off, they go from where they are kept
    assert!(!dictionaries.exists());
}

#[test]
fn a_host_has_a_kept_cover_from_the_folder_it_gave() {
    let cx = &TestExecutor::new();
    let (_dir, store) = store_with("romaji_search = false\n");
    let cache = tempfile::tempdir().unwrap();
    let hiphile = hiphile_keeping(Ok(store), Caches::At(cache.path().to_owned()), cx);
    let url = "http://127.0.0.1:9/rest/getCoverArt?id=al-1&size=80";
    let cover = subsonic::CoverArtRequest::parse(&url::Url::parse(url).unwrap()).unwrap();
    hiphile.covers().unwrap().put(&cover, b"image").unwrap();

    let mut image = hiphile.cover_image(url);
    cx.run_until_parked();

    let image = futures::FutureExt::now_or_never(&mut image).flatten();
    assert_eq!(image.as_deref(), Some(&b"image"[..]));
}

#[test]
fn a_cover_neither_kept_nor_on_the_server_is_none() {
    let cx = &TestExecutor::new();
    // The server's answer comes from the network's own thread
    cx.allow_parking();
    let (_dir, store) = store_with("romaji_search = false\n");
    let hiphile = hiphile(Ok(store), cx);

    let mut image = hiphile.cover_image("http://127.0.0.1:9/rest/getCoverArt?id=al-1&size=80");
    cx.run_until_parked();

    assert_eq!(futures::FutureExt::now_or_never(&mut image), Some(None));
}

fn cover_file(hiphile: &Hiphile, url: &str, cx: &TestExecutor) -> Option<CoverSource> {
    let mut file = hiphile.cover_file(url);
    let mut found = None;
    wait_for(cx, || {
        found = futures::FutureExt::now_or_never(&mut file);
        found.is_some()
    });
    found.flatten()
}

#[test]
fn a_cover_fetched_is_given_as_the_file_it_is_kept_in() {
    let cx = &TestExecutor::new();
    cx.allow_parking();
    let (_dir, store) = store_with("romaji_search = false\n");
    let cache = tempfile::tempdir().unwrap();
    let hiphile = hiphile_keeping(Ok(store), Caches::At(cache.path().to_owned()), cx);
    let url = format!(
        "{}/rest/getCoverArt?id=al-1&size=80",
        server_giving_image(b"jpeg")
    );

    let Some(CoverSource::File(path)) = cover_file(&hiphile, &url, cx) else {
        panic!("the cover was not kept");
    };
    assert_eq!(std::fs::read(&path).unwrap(), b"jpeg");
    assert_eq!(
        cover_file(&hiphile, &url, cx),
        Some(CoverSource::File(path))
    );
}

#[test]
fn a_cover_fetched_again_without_caches_is_the_image_still_held() {
    let cx = &TestExecutor::new();
    cx.allow_parking();
    let (_dir, store) = store_with("romaji_search = false\n");
    let hiphile = hiphile(Ok(store), cx);
    let url = format!(
        "{}/rest/getCoverArt?id=al-1&size=80",
        server_giving_image(b"jpeg")
    );

    let Some(CoverSource::Bytes(first)) = cover_file(&hiphile, &url, cx) else {
        panic!("the cover did not come from the server");
    };
    let Some(CoverSource::Bytes(again)) = cover_file(&hiphile, &url, cx) else {
        panic!("the cover did not come from the server");
    };
    assert_eq!(&first[..], b"jpeg");
    assert!(Arc::ptr_eq(&first, &again));
}

#[test]
fn the_dictionaries_are_left_as_they_are_until_the_readings_are_needed() {
    let cx = &TestExecutor::new();
    cx.allow_parking();
    let (_dir, store) = store_with("romaji_search = false\n");
    let cache = tempfile::tempdir().unwrap();
    let dictionaries = cache.path().join("readings");
    std::fs::create_dir(&dictionaries).unwrap();

    let hiphile = hiphile_keeping(Ok(store), Caches::At(cache.path().to_owned()), cx);
    let player = hiphile.player(crate::test_support::session(crate::testing::NOWHERE));
    cx.run_until_parked();
    assert!(dictionaries.exists());
    player.shutdown();

    hiphile.open_workspace(
        crate::test_support::session(crate::testing::NOWHERE),
        config::Profile::new(crate::testing::NOWHERE, "alice"),
    );
    cx.run_until_parked();
    assert!(!dictionaries.exists());
    let workspace = hiphile.workspace().unwrap();
    assert!(workspace.session().readings().is_some());
    workspace.shutdown();
    cx.run_until_parked();
}

#[test]
fn the_songs_kept_in_the_folder_given_are_counted_and_cleared() {
    let cx = &TestExecutor::new();
    let (_dir, store) = store_with("romaji_search = false\n");
    let cache = tempfile::tempdir().unwrap();
    let hiphile = hiphile_keeping(Ok(store), Caches::At(cache.path().to_owned()), cx);
    let server = url::Url::parse(crate::testing::NOWHERE).unwrap();
    catalog::SongStore::in_cache(cache.path(), catalog::SONG_CAPACITY)
        .put(&server, &subsonic::SongId::new("so-1"), &[0; 100])
        .unwrap();

    let usage = hiphile.song_cache_usage();
    cx.run_until_parked();
    assert_eq!(usage.snapshot().kept(), Some(100));

    usage.clear();
    cx.run_until_parked();
    assert_eq!(usage.snapshot().kept(), Some(0));
}

#[test]
fn with_nowhere_to_keep_them_no_songs_are_counted() {
    let cx = &TestExecutor::new();
    let (_dir, store) = store_with("romaji_search = false\n");
    let hiphile = hiphile(Ok(store), cx);

    let usage = hiphile.song_cache_usage();
    cx.run_until_parked();

    assert_eq!(usage.snapshot().kept(), None);
}

#[test]
fn songs_are_kept_once_a_limit_replaces_one_that_could_not_be_read() {
    let cx = &TestExecutor::new();
    let (_dir, store) = store_with("song_cache_gib = -1\nromaji_search = false\n");
    let cache = tempfile::tempdir().unwrap();
    let hiphile = hiphile_keeping(Ok(store), Caches::At(cache.path().to_owned()), cx);
    let is_keeping = || hiphile.0.state.read(|state| state.song_cache.is_some());
    assert!(!is_keeping());

    hiphile.settings().song_cache().set_gib(1.0);
    cx.advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();

    assert!(is_keeping());
}

#[test]
fn a_player_without_a_workspace_plays_at_the_saved_volume() {
    let cx = &TestExecutor::new();
    cx.allow_parking();
    let (_dir, store) = store_with("volume = 0.5\nromaji_search = false\n");
    let hiphile = hiphile(Ok(store), cx);

    let player = hiphile.player(crate::test_support::session(crate::testing::NOWHERE));

    assert_eq!(player.snapshot().volume(), 0.5);
    assert!(hiphile.workspace().is_none());
    cx.run_until_parked();
}

#[test]
fn a_player_without_a_workspace_plays_with_the_saved_dsp() {
    let cx = &TestExecutor::new();
    cx.allow_parking();
    let (_dir, store) = store_with("crossfeed = true\nbalance_db = -2.0\nromaji_search = false\n");
    let hiphile = hiphile(Ok(store), cx);

    let player = hiphile.player(crate::test_support::session(crate::testing::NOWHERE));

    let dsp = player.dsp();
    assert_eq!(dsp.crossfeed(), audio::Crossfeed::new(700, 4.5));
    assert_eq!(dsp.balance(), audio::Balance::from_db(-2.0));
    cx.run_until_parked();
}

#[test]
fn signing_out_closes_the_workspace() {
    let cx = &TestExecutor::new();
    cx.allow_parking();
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

#[test]
fn what_was_typed_is_checked_before_signing_in() {
    let cx = &TestExecutor::new();
    let hiphile = hiphile(Err(config::Error::NoHomeDir), cx);

    let Err(SignInRefused::Invalid(invalid)) = hiphile.sign_in("", "alice", "secret") else {
        panic!("signing in started");
    };

    assert_eq!(invalid.field(), session::SignInField::Server);
    assert!(hiphile.workspace().is_none());
    assert!(!hiphile.snapshot().is_signing_in());
    cx.run_until_parked();
}

#[test]
fn another_attempt_waits_for_the_one_under_way() {
    let cx = &TestExecutor::new();
    // The server's answer comes from the network's own thread
    cx.allow_parking();
    let hiphile = hiphile(Err(config::Error::NoHomeDir), cx);

    let mut first = hiphile.sign_in(NOWHERE, "alice", "secret").unwrap();
    assert!(hiphile.snapshot().is_signing_in());
    let second = hiphile.sign_in(NOWHERE, "bob", "secret");
    assert_eq!(second.err(), Some(SignInRefused::SigningIn));

    wait_for(cx, || !hiphile.snapshot().is_signing_in());
    let error = futures::FutureExt::now_or_never(&mut first)
        .unwrap()
        .unwrap_err();
    assert!(error.starts_with("Couldn't reach"), "{error}");
    let third = hiphile.sign_in(NOWHERE, "bob", "secret");
    assert!(third.is_ok());
    drop(third);
    cx.run_until_parked();
    assert!(!hiphile.snapshot().is_signing_in());
}

#[test]
fn an_attempt_dropped_unfinished_ends() {
    let cx = &TestExecutor::new();
    let hiphile = hiphile(Err(config::Error::NoHomeDir), cx);
    let attempt = hiphile.sign_in(NOWHERE, "alice", "secret").unwrap();

    drop(attempt);
    cx.run_until_parked();

    assert!(!hiphile.snapshot().is_signing_in());
}

fn store_with_alice() -> (TempDir, config::Store) {
    store_with(&format!(
        "server = \"{NOWHERE}\"\nusername = \"alice\"\npassword = \"secret\"\n\
         romaji_search = false\n"
    ))
}

#[test]
fn whoever_signed_in_last_is_signed_in_again_only_once() {
    let cx = &TestExecutor::new();
    // The server's answer comes from the network's own thread
    cx.allow_parking();
    let (_dir, store) = store_with_alice();
    let hiphile = hiphile(Ok(store), cx);

    let mut resumed = hiphile.resume().expect("nothing to resume");
    assert!(hiphile.resume().is_none());

    let mut result = None;
    wait_for(cx, || {
        result = futures::FutureExt::now_or_never(&mut resumed);
        result.is_some()
    });
    // With the password saved, as nothing answers there
    let error = result.unwrap().unwrap_err();
    assert!(error.starts_with("Couldn't reach"), "{error}");
    assert!(!hiphile.snapshot().is_signing_in());
}

#[test]
fn nothing_is_resumed_after_signing_out() {
    let cx = &TestExecutor::new();
    let (_dir, store) = store_with_alice();
    let hiphile = hiphile(Ok(store), cx);

    hiphile.sign_out();

    assert!(hiphile.resume().is_none());
    cx.run_until_parked();
}

#[test]
fn nothing_is_resumed_for_someone_new() {
    let cx = &TestExecutor::new();
    let (_dir, store) = store_with("romaji_search = false\n");
    let hiphile = hiphile(Ok(store), cx);

    assert!(hiphile.resume().is_none());
}

#[test]
fn dictionaries_that_could_not_be_had_are_told_to_a_host_that_follows_later() {
    let cx = &TestExecutor::new();
    let (_dir, store) = store_with("");
    let hiphile = hiphile(Ok(store), cx);
    assert!(hiphile.alerts().earlier.is_empty());
    assert!(hiphile.0.readings.get().is_none());

    hiphile.readings();
    cx.run_until_parked();

    let earlier = hiphile.alerts().earlier;
    assert_eq!(earlier.len(), 1);
    assert!(earlier[0].message.ends_with(" Try again in Settings."));
    assert_eq!(earlier[0].told_on, None);
}

fn alice() -> config::Profile {
    config::Profile::new(NOWHERE, "alice")
}

fn bob() -> config::Profile {
    config::Profile::new(NOWHERE, "bob")
}

/// Signed in with alice and bob before, alice last.
fn store_with_alice_and_bob() -> (TempDir, config::Store) {
    let (dir, store) = store_with("romaji_search = false\n");
    store.save_profile(&bob()).unwrap();
    store.save_profile(&alice()).unwrap();
    (dir, store)
}

fn hiphile_with_keychain(
    store: config::Store,
    keychain: &config::Keychain,
    cx: &TestExecutor,
) -> Hiphile {
    let executor = cx.executor();
    let net = net::Client::new("hiphile-test").unwrap();
    Hiphile::new(
        executor,
        net,
        Ok(store),
        Some(keychain.clone()),
        Caches::None,
    )
}

fn ended<T>(task: &mut Task<T>, cx: &TestExecutor) -> T {
    let mut result = None;
    wait_for(cx, || {
        result = futures::FutureExt::now_or_never(&mut *task);
        result.is_some()
    });
    result.unwrap()
}

#[test]
fn the_profiles_signed_in_with_before_are_there_from_the_start() {
    let cx = &TestExecutor::new();
    let (_dir, store) = store_with_alice_and_bob();

    let hiphile = hiphile(Ok(store), cx);

    assert_eq!(hiphile.snapshot().profiles(), [bob(), alice()]);
}

#[test]
fn switching_to_whoever_is_signed_in_leaves_their_workspace() {
    let cx = &TestExecutor::new();
    // The workspace's requests answer from the network's own thread
    cx.allow_parking();
    let (_dir, store) = store_with_alice_and_bob();
    let hiphile = hiphile(Ok(store), cx);
    hiphile.open_workspace(crate::test_support::session(NOWHERE), alice());
    let workspace = hiphile.workspace();

    let mut switching = hiphile.switch_to(alice());

    assert_eq!(
        futures::FutureExt::now_or_never(&mut switching),
        Some(Ok(()))
    );
    assert!(!hiphile.snapshot().is_signing_in());
    assert_eq!(hiphile.workspace(), workspace);
    workspace.unwrap().shutdown();
    cx.run_until_parked();
}

#[test]
fn switching_to_someone_without_a_saved_password_asks_for_one() {
    let cx = &TestExecutor::new();
    // The workspace's requests answer from the network's own thread
    cx.allow_parking();
    let (_dir, store) = store_with_alice_and_bob();
    let hiphile = hiphile_with_keychain(store, &config::Keychain::in_memory(), cx);
    hiphile.open_workspace(crate::test_support::session(NOWHERE), alice());

    let mut switching = hiphile.switch_to(bob());

    assert_eq!(ended(&mut switching, cx), Err(SwitchFailed::NoPassword));
    assert_eq!(hiphile.snapshot().profile(), Some(&alice()));
    hiphile.workspace().unwrap().shutdown();
    cx.run_until_parked();
}

#[test]
fn switching_signs_in_with_the_saved_password_while_the_workspace_stays() {
    let cx = &TestExecutor::new();
    // The server's answer comes from the network's own thread
    cx.allow_parking();
    let (_dir, store) = store_with_alice_and_bob();
    let keychain = config::Keychain::in_memory();
    keychain.set_password(&bob(), "secret").unwrap();
    let hiphile = hiphile_with_keychain(store, &keychain, cx);
    hiphile.open_workspace(crate::test_support::session(NOWHERE), alice());

    let mut switching = hiphile.switch_to(bob());

    // With the password saved, as nothing answers there
    let Err(SwitchFailed::SignIn(error)) = ended(&mut switching, cx) else {
        panic!("signing in was not tried");
    };
    assert!(error.starts_with("Couldn't reach"), "{error}");
    assert_eq!(hiphile.snapshot().profile(), Some(&alice()));
    assert!(!hiphile.snapshot().is_signing_in());
    hiphile.workspace().unwrap().shutdown();
    cx.run_until_parked();
}

#[test]
fn signing_out_takes_the_profile_off_the_others() {
    let cx = &TestExecutor::new();
    cx.allow_parking();
    let (_dir, store) = store_with_alice_and_bob();
    let hiphile = hiphile(Ok(store.clone()), cx);
    hiphile.open_workspace(crate::test_support::session(NOWHERE), alice());

    hiphile.sign_out();

    assert_eq!(hiphile.snapshot().profiles(), [bob()]);
    cx.run_until_parked();
    assert_eq!(store.profiles().unwrap(), [bob()]);
}

#[test]
fn signing_out_calls_off_a_switch_under_way() {
    let cx = &TestExecutor::new();
    cx.allow_parking();
    let (_dir, store) = store_with_alice_and_bob();
    let carol = config::Profile::new(server_never_answering(), "carol");
    let keychain = config::Keychain::in_memory();
    keychain.set_password(&carol, "secret").unwrap();
    let hiphile = hiphile_with_keychain(store, &keychain, cx);
    hiphile.open_workspace(crate::test_support::session(NOWHERE), alice());
    let mut switching = hiphile.switch_to(carol);
    wait_for(cx, || hiphile.snapshot().is_signing_in());

    hiphile.sign_out();

    assert!(!hiphile.snapshot().is_signing_in());
    assert_eq!(ended(&mut switching, cx), Ok(()));
    assert_eq!(hiphile.workspace(), None);
    assert_eq!(hiphile.snapshot().profiles(), [bob()]);
}

// Each seed finds the two passwords in another order
#[test]
fn a_switch_asked_for_while_resuming_finds_its_password_takes_its_place() {
    for seed in 0..30 {
        let cx = &TestExecutor::with_seed(seed);
        cx.allow_parking();
        let (_dir, store) = store_with("romaji_search = false\n");
        let carol = config::Profile::new(server_never_answering(), "carol");
        store.save_profile(&bob()).unwrap();
        store.save_profile(&carol).unwrap();
        let keychain = config::Keychain::in_memory();
        keychain.set_password(&carol, "secret").unwrap();
        keychain.set_password(&bob(), "secret").unwrap();
        let hiphile = hiphile_with_keychain(store, &keychain, cx);

        let mut resuming = hiphile.resume().unwrap();
        let mut switching = hiphile.switch_to(bob());

        assert_eq!(ended(&mut resuming, cx), Ok(()));
        // Tried with bob's password, as nothing answers there
        let Err(SwitchFailed::SignIn(error)) = ended(&mut switching, cx) else {
            panic!("signing in as bob was not tried");
        };
        assert!(error.starts_with("Couldn't reach"), "{error}");
    }
}

#[test]
fn a_profile_forgotten_is_off_the_others_with_its_password_unless_signed_in() {
    let cx = &TestExecutor::new();
    cx.allow_parking();
    let (_dir, store) = store_with_alice_and_bob();
    let keychain = config::Keychain::in_memory();
    keychain.set_password(&bob(), "secret").unwrap();
    let hiphile = hiphile_with_keychain(store.clone(), &keychain, cx);
    hiphile.open_workspace(crate::test_support::session(NOWHERE), alice());

    hiphile.forget(alice());
    hiphile.forget(bob());

    assert_eq!(hiphile.snapshot().profiles(), [alice()]);
    cx.run_until_parked();
    assert_eq!(store.profiles().unwrap(), [alice()]);
    assert_eq!(keychain.password(&bob()).unwrap(), None);
    hiphile.workspace().unwrap().shutdown();
    cx.run_until_parked();
}
