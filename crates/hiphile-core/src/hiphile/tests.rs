use gpui_kit::TestAppContext;
use tempfile::TempDir;

use super::*;
use crate::testing::{NOWHERE, wait_for};

fn hiphile(store: Result<config::Store, config::Error>, cx: &mut TestAppContext) -> Hiphile {
    hiphile_keeping(store, Caches::None, cx)
}

fn hiphile_keeping(
    store: Result<config::Store, config::Error>,
    caches: Caches,
    cx: &mut TestAppContext,
) -> Hiphile {
    let executor = cx.update(|cx| gpui_host::executor(cx));
    let net = net::Client::new("hiphile-test").unwrap();
    Hiphile::new(executor, net, store, None, caches)
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
fn a_folder_the_host_gives_keeps_the_covers_and_the_dictionaries(cx: &mut TestAppContext) {
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

#[gpui_kit::test]
fn a_host_has_a_kept_cover_from_the_folder_it_gave(cx: &mut TestAppContext) {
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

#[gpui_kit::test]
fn a_cover_neither_kept_nor_on_the_server_is_none(cx: &mut TestAppContext) {
    // The server's answer comes from the network's own thread
    cx.executor().allow_parking();
    let (_dir, store) = store_with("romaji_search = false\n");
    let hiphile = hiphile(Ok(store), cx);

    let mut image = hiphile.cover_image("http://127.0.0.1:9/rest/getCoverArt?id=al-1&size=80");
    cx.run_until_parked();

    assert_eq!(futures::FutureExt::now_or_never(&mut image), Some(None));
}

#[gpui_kit::test]
fn the_dictionaries_are_left_as_they_are_until_the_readings_are_needed(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
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
}

#[gpui_kit::test]
fn the_songs_kept_in_the_folder_given_are_counted_and_cleared(cx: &mut TestAppContext) {
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

#[gpui_kit::test]
fn with_nowhere_to_keep_them_no_songs_are_counted(cx: &mut TestAppContext) {
    let (_dir, store) = store_with("romaji_search = false\n");
    let hiphile = hiphile(Ok(store), cx);

    let usage = hiphile.song_cache_usage();
    cx.run_until_parked();

    assert_eq!(usage.snapshot().kept(), None);
}

#[gpui_kit::test]
fn songs_are_kept_once_a_limit_replaces_one_that_could_not_be_read(cx: &mut TestAppContext) {
    let (_dir, store) = store_with("song_cache_gib = -1\nromaji_search = false\n");
    let cache = tempfile::tempdir().unwrap();
    let hiphile = hiphile_keeping(Ok(store), Caches::At(cache.path().to_owned()), cx);
    let is_keeping = || hiphile.0.state.read(|state| state.song_cache.is_some());
    assert!(!is_keeping());

    hiphile.settings().song_cache().set_gib(1.0);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();

    assert!(is_keeping());
}

#[gpui_kit::test]
fn a_player_without_a_workspace_plays_at_the_saved_volume(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (_dir, store) = store_with("volume = 0.5\nromaji_search = false\n");
    let hiphile = hiphile(Ok(store), cx);

    let player = hiphile.player(crate::test_support::session(crate::testing::NOWHERE));

    assert_eq!(player.snapshot().volume(), 0.5);
    assert!(hiphile.workspace().is_none());
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_player_without_a_workspace_plays_with_the_saved_dsp(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (_dir, store) = store_with("crossfeed = true\nbalance_db = -2.0\nromaji_search = false\n");
    let hiphile = hiphile(Ok(store), cx);

    let player = hiphile.player(crate::test_support::session(crate::testing::NOWHERE));

    let dsp = player.dsp();
    assert_eq!(dsp.crossfeed(), audio::Crossfeed::new(700, 4.5));
    assert_eq!(dsp.balance(), audio::Balance::from_db(-2.0));
    cx.run_until_parked();
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

    let Err(SignInRefused::Invalid(invalid)) = hiphile.sign_in("", "alice", "secret") else {
        panic!("signing in started");
    };

    assert_eq!(invalid.field(), session::SignInField::Server);
    assert!(hiphile.workspace().is_none());
    assert!(!hiphile.snapshot().is_signing_in());
    cx.run_until_parked();
}

#[gpui_kit::test]
fn another_attempt_waits_for_the_one_under_way(cx: &mut TestAppContext) {
    // The server's answer comes from the network's own thread
    cx.executor().allow_parking();
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

#[gpui_kit::test]
fn an_attempt_dropped_unfinished_ends(cx: &mut TestAppContext) {
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

#[gpui_kit::test]
fn whoever_signed_in_last_is_signed_in_again_only_once(cx: &mut TestAppContext) {
    // The server's answer comes from the network's own thread
    cx.executor().allow_parking();
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

#[gpui_kit::test]
fn nothing_is_resumed_after_signing_out(cx: &mut TestAppContext) {
    let (_dir, store) = store_with_alice();
    let hiphile = hiphile(Ok(store), cx);

    hiphile.sign_out();

    assert!(hiphile.resume().is_none());
    cx.run_until_parked();
}

#[gpui_kit::test]
fn nothing_is_resumed_for_someone_new(cx: &mut TestAppContext) {
    let (_dir, store) = store_with("romaji_search = false\n");
    let hiphile = hiphile(Ok(store), cx);

    assert!(hiphile.resume().is_none());
}

#[gpui_kit::test]
fn dictionaries_that_could_not_be_had_are_told_to_a_host_that_follows_later(
    cx: &mut TestAppContext,
) {
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
