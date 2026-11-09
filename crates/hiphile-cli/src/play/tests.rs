use hiphile_core::test_support::session;
use hiphile_core::{Caches, Hiphile};
use serde_json::json;
use tempfile::TempDir;
use tokio::runtime::{Builder, Runtime};

use super::*;

/// A server that cannot be reached, so no song opens and nothing reaches an output device.
const NOWHERE: &str = "http://127.0.0.1:9";

/// The core on a runtime with a paused clock, as the CLI runs it, with a store of its own.
struct Test {
    runtime: Runtime,
    hiphile: Hiphile,
    _store: TempDir,
}

impl Test {
    fn new() -> Self {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
            .unwrap();
        let store = tempfile::tempdir().unwrap();
        let hiphile = Hiphile::new(
            Executor::tokio(runtime.handle().clone()),
            net::Client::new("hiphile-test").unwrap(),
            Ok(config::Store::at(store.path())),
            None,
            Caches::None,
        );
        Self {
            runtime,
            hiphile,
            _store: store,
        }
    }

    fn player(&self) -> Player {
        self.hiphile.player(session(NOWHERE))
    }
}

fn song(title: &str) -> Song {
    serde_json::from_value(json!({
        "id": title,
        "title": title,
        "artist": "Artist",
        "duration": 100,
    }))
    .unwrap()
}

/// What `report` says of `player` as it stands, at `position`.
fn look(report: &mut Report, player: &Player, position: Duration) -> (Step, String, String) {
    let (mut out, mut progress) = (Vec::new(), Vec::new());
    let step = report
        .look(&player.snapshot(), position, &mut out, &mut progress)
        .unwrap();
    let text = |bytes| String::from_utf8(bytes).unwrap();
    (step, text(out), text(progress))
}

#[test]
fn a_song_is_named_once_and_its_position_shown_while_it_plays() {
    let test = Test::new();
    let player = test.player();
    player.fill_queue(&[song("a"), song("b")], 0);
    player.pretend_playing(true);
    let mut report = Report::default();

    let first = look(&mut report, &player, Duration::from_secs(61));
    let second = look(&mut report, &player, Duration::from_secs(62));

    assert_eq!(
        first,
        (
            Step::Wait,
            "Artist - a\n".to_owned(),
            "\r  1:01 / 1:40".to_owned()
        )
    );
    assert_eq!(
        second,
        (Step::Wait, String::new(), "\r  1:02 / 1:40".to_owned())
    );
}

#[test]
fn a_failed_song_ends_the_line_is_told_and_asks_for_the_next() {
    let test = Test::new();
    let player = test.player();
    player.fill_queue(&[song("a"), song("b")], 0);
    let mut report = Report::default();
    look(&mut report, &player, Duration::ZERO);

    player.set_error("Couldn't play “a”. It broke.");
    let (step, out, progress) = look(&mut report, &player, Duration::ZERO);

    assert_eq!(step, Step::Next);
    assert_eq!(out, "");
    assert_eq!(progress, "\na: Couldn't play “a”. It broke.\n");
    assert_eq!(report.failures, 1);
}

#[test]
fn the_end_of_the_queue_ends_the_line_and_the_playing() {
    let test = Test::new();
    let player = test.player();
    player.fill_queue(&[song("a")], 0);
    player.pretend_playing(true);
    let mut report = Report::default();
    look(&mut report, &player, Duration::ZERO);

    player.fill_queue(&[], 0);
    let (step, out, progress) = look(&mut report, &player, Duration::ZERO);

    assert_eq!(
        (step, out, progress),
        (Step::Done, String::new(), "\n".to_owned())
    );
}

#[test]
fn the_cores_player_plays_past_every_song_that_cannot_be_opened() {
    let test = Test::new();
    let player = test.player();
    let executor = Executor::tokio(test.runtime.handle().clone());
    let (mut out, mut progress) = (Vec::new(), Vec::new());

    let failures = test
        .runtime
        .block_on(play_through(
            &player,
            &executor,
            &[song("a"), song("b")],
            &mut out,
            &mut progress,
        ))
        .unwrap();

    assert_eq!(failures, 2);
    assert_eq!(String::from_utf8(out).unwrap(), "Artist - a\nArtist - b\n");
    let progress = String::from_utf8(progress).unwrap();
    for title in ["a", "b"] {
        let told = format!("{title}: Couldn't play “{title}”. Couldn't reach the server.");
        assert!(progress.contains(&told), "{progress}");
    }
    assert!(!player.snapshot().has_song());
}
