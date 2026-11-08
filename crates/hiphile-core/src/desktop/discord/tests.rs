use gpui_kit::TestAppContext;

use super::*;
use crate::testing::NOWHERE;

fn status(cx: &mut TestAppContext) -> (DiscordStatus, playback::Player) {
    let executor = cx.update(|cx| gpui_host::executor(cx));
    let player = playback::Player::new(
        executor.clone(),
        crate::test_support::session(NOWHERE),
        Err(config::Error::NoHomeDir),
        audio::OutputSettings::default(),
        None,
    );
    let status = DiscordStatus::new(
        executor,
        player.clone(),
        crate::test_support::session(NOWHERE),
        None,
        Presence::disconnected(),
    );
    (status, player)
}

fn song(id: &str) -> Song {
    serde_json::from_value(serde_json::json!({
        "id": id, "title": id, "duration": 100, "coverArt": format!("cover-{id}"),
    }))
    .unwrap()
}

#[gpui_kit::test]
fn the_status_follows_the_song_only_while_it_plays(cx: &mut TestAppContext) {
    let (status, player) = status(cx);
    let is_refreshing = || status.lock()._refresh.is_some();
    player.fill_queue(&[song("a")], 0);
    cx.run_until_parked();
    assert!(!is_refreshing());

    player.pretend_playing(true);
    cx.run_until_parked();
    assert!(is_refreshing());

    player.pretend_playing(false);
    cx.run_until_parked();
    assert!(!is_refreshing());
}

#[gpui_kit::test]
fn a_cover_is_looked_for_only_with_a_key_and_while_on(cx: &mut TestAppContext) {
    let (status, player) = status(cx);
    let sought = || status.lock().sought_cover.clone();
    player.fill_queue(&[song("a")], 0);
    player.pretend_playing(true);
    cx.run_until_parked();
    assert_eq!(sought(), None, "no key");

    status.set_on(false);
    status.set_cover_key(Some("key"));
    status.show(&player.snapshot());
    assert_eq!(sought(), None, "off");

    status.set_on(true);
    status.show(&player.snapshot());
    assert_eq!(sought(), Some(CoverArtId::new("cover-a")));

    status.set_cover_key(Some("another"));
    assert_eq!(sought(), None, "a new key tries again");
}
