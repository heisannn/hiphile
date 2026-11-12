use std::time::Duration;

use executor::testing::TestExecutor;

use super::*;
use crate::testing::NOWHERE;

fn controls(cx: &TestExecutor) -> (SystemMediaControls, playback::Player) {
    let executor = cx.executor();
    let player = playback::Player::new(
        executor.clone(),
        crate::test_support::session(NOWHERE),
        Err(config::Error::NoHomeDir),
        audio::OutputSettings::default(),
        None,
    );
    let controls = SystemMediaControls::new(
        executor,
        player.clone(),
        crate::test_support::session(NOWHERE),
        None,
        MediaControls::disconnected(),
    );
    (controls, player)
}

fn named_song(id: &str) -> Song {
    serde_json::from_value(serde_json::json!({ "id": id, "title": id, "duration": 100 })).unwrap()
}

fn current_title(player: &playback::Player) -> Option<String> {
    player
        .snapshot()
        .current_song()
        .map(|song| song.title().to_owned())
}

#[test]
fn play_and_pause_from_the_system_only_change_what_they_name() {
    let cx = &TestExecutor::new();
    // Playing again tells the server, whose answer comes from the network's thread
    cx.allow_parking();
    let (controls, player) = controls(cx);
    player.fill_queue(&[named_song("a")], 0);
    player.pretend_playing(true);
    let is_playing = || player.snapshot().is_playing();

    controls.on_command(MediaCommand::Play);
    assert!(is_playing());
    controls.on_command(MediaCommand::Pause);
    assert!(!is_playing());
    controls.on_command(MediaCommand::Pause);
    assert!(!is_playing());
    controls.on_command(MediaCommand::Play);
    assert!(is_playing());
    controls.on_command(MediaCommand::Toggle);
    assert!(!is_playing());
}

#[test]
fn next_and_previous_from_the_system_move_in_the_queue() {
    let cx = &TestExecutor::new();
    let (controls, player) = controls(cx);
    player.fill_queue(&[named_song("a"), named_song("b")], 0);
    controls.on_command(MediaCommand::Next);
    assert_eq!(current_title(&player).as_deref(), Some("b"));
    controls.on_command(MediaCommand::Previous);
    assert_eq!(current_title(&player).as_deref(), Some("a"));
}

#[test]
fn stop_from_the_system_keeps_the_song_for_what_is_added_next() {
    let cx = &TestExecutor::new();
    let (controls, player) = controls(cx);
    player.fill_queue(&[named_song("a"), named_song("b")], 0);
    player.pretend_playing(true);
    controls.on_command(MediaCommand::Stop);
    assert!(!player.snapshot().is_playing());

    player.enqueue(&[named_song("c")], playback::Placement::Last);
    assert_eq!(current_title(&player).as_deref(), Some("a"));
    assert!(!player.snapshot().is_playing());
}

#[test]
fn seeks_from_the_system_stay_in_the_song_until_its_end() {
    let cx = &TestExecutor::new();
    let (controls, player) = controls(cx);
    player.fill_queue(&[named_song("a"), named_song("b")], 0);
    player.pretend_playing(true);
    let seconds = Duration::from_secs;

    controls.on_command(MediaCommand::SeekTo(seconds(50)));
    controls.on_command(MediaCommand::SeekForward(None));
    controls.on_command(MediaCommand::SeekForward(Some(seconds(20))));
    controls.on_command(MediaCommand::SeekBackward(None));
    assert_eq!(current_title(&player).as_deref(), Some("a"));

    controls.on_command(MediaCommand::SeekTo(seconds(100)));
    assert_eq!(current_title(&player).as_deref(), Some("b"));
}

#[test]
fn raising_the_window_is_left_to_the_app() {
    let cx = &TestExecutor::new();
    let (controls, _player) = controls(cx);
    let mut notices = controls.notices();
    controls.on_command(MediaCommand::Raise);
    assert_eq!(notices.try_recv(), Ok(MediaNotice::Raise));
}

#[test]
fn the_system_hears_how_the_song_stands() {
    let cx = &TestExecutor::new();
    let (_controls, player) = controls(cx);
    player.fill_queue(&[named_song("a")], 0);
    assert_eq!(media_status(&player.snapshot()), MediaStatus::Stopped);

    player.pretend_playing(true);
    assert_eq!(media_status(&player.snapshot()), MediaStatus::Playing);
    player.pretend_playing(false);
    assert_eq!(media_status(&player.snapshot()), MediaStatus::Paused);
}

#[test]
fn a_cover_not_found_is_looked_for_again_with_the_next_song() {
    let cx = &TestExecutor::new();
    let (controls, player) = controls(cx);
    player.fill_queue(&[named_song("a"), named_song("b")], 0);
    cx.run_until_parked();
    controls.lock().sought_cover = Some(CoverArtId::new("c"));
    player.next();
    cx.run_until_parked();
    assert_eq!(controls.lock().sought_cover, None);
}

#[test]
fn letting_go_of_them_lets_go_of_nothing_else() {
    let cx = &TestExecutor::new();
    let (controls, player) = controls(cx);
    let weak = controls.downgrade();
    drop(controls);
    cx.run_until_parked();
    assert!(weak.upgrade().is_none());
    drop(player);
}

#[test]
fn shutting_the_core_down_clears_what_the_system_shows() {
    let cx = &TestExecutor::new();
    cx.allow_parking();
    let net = net::Client::new("hiphile-test").unwrap();
    let executor = cx.executor();
    let hiphile = crate::Hiphile::new(
        executor,
        net,
        Err(config::Error::NoHomeDir),
        None,
        crate::Caches::None,
    );
    hiphile.open_workspace(
        crate::test_support::session(NOWHERE),
        config::Profile::new(NOWHERE, "alice"),
    );
    let controls = hiphile
        .workspace()
        .unwrap()
        .connect_media_controls(MediaControls::disconnected());
    let cover = CoverArtId::new("c");
    let file = CoverFile::write(b"image").unwrap();
    controls.lock().controls.set_cover(cover.clone(), file);

    hiphile.shutdown();

    assert!(!controls.lock().controls.has_cover(&cover));
    cx.run_until_parked();
}
