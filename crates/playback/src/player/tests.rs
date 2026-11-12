use std::collections::HashMap;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use executor::testing::TestExecutor;
use serde_json::json;
use subsonic::Credentials;

use super::*;

const RATE: u32 = 44_100;
const WAIT: Duration = Duration::from_secs(15);

fn session(url: &str) -> session::Session {
    let net = net::Client::new("hiphile-test").unwrap();
    let server = client::Server::new(url, Credentials::api_key("k")).unwrap();
    let info = serde_json::from_value(json!({ "version": "1.16.1" })).unwrap();
    session::Session::new(
        client::Client::new(net, server),
        "alice".to_owned(),
        info,
        None,
        None,
    )
}

/// A player that plays nothing, so nothing reaches a device or the server.
fn idle_player(cx: &TestExecutor) -> Player {
    let executor = cx.executor();
    Player::new(
        executor,
        session("http://127.0.0.1:9"),
        Err(config::Error::NoHomeDir),
        OutputSettings::default(),
        None,
    )
}

fn named_song(id: &str) -> Song {
    serde_json::from_value(json!({ "id": id, "title": id, "duration": 100 })).unwrap()
}

fn queued_titles(player: &Player) -> Vec<String> {
    let songs = player.snapshot().queued_songs();
    songs.iter().map(|song| song.title().to_owned()).collect()
}

fn current_title(player: &Player) -> Option<String> {
    player
        .snapshot()
        .current_song()
        .map(|song| song.title().to_owned())
}

#[test]
fn skipped_songs_stay_out_of_the_queue() {
    let cx = &TestExecutor::new();
    let player = idle_player(cx);
    player.set_skip_instrumentals(true);
    let inst = named_song("a [instrumental]");
    let vocal = named_song("b");
    let other = named_song("c");
    let songs = [inst.clone(), vocal.clone(), inst.clone(), other.clone()];

    player.fill_queue(std::slice::from_ref(&vocal), 0);
    player.pretend_playing(true);
    assert!(player.enqueue(&songs, Placement::Last));
    assert_eq!(queued_titles(&player), ["b", "b", "c"]);

    assert!(!player.enqueue(std::slice::from_ref(&inst), Placement::Next));
    assert!(!player.play(&songs, 2));
    assert!(!player.play_whole(std::slice::from_ref(&inst)));
    assert_eq!(queued_titles(&player), ["b", "b", "c"]);

    assert!(player.play(&songs, 3));
    assert_eq!(queued_titles(&player), ["b", "c"]);
    assert_eq!(current_title(&player).as_deref(), Some("c"));

    assert!(player.play_whole(&songs));
    assert_eq!(queued_titles(&player), ["b", "c"]);
}

#[test]
fn each_command_left_with_nothing_but_skipped_songs_says_so() {
    let cx = &TestExecutor::new();
    let player = idle_player(cx);
    player.set_skip_instrumentals(true);
    let inst = named_song("a [instrumental]");
    player.fill_queue(&[named_song("b")], 0);
    player.pretend_playing(true);
    let mut notices = player.notices();

    player.play(std::slice::from_ref(&inst), 0);
    player.play_whole(std::slice::from_ref(&inst));
    player.enqueue(std::slice::from_ref(&inst), Placement::Last);
    player.enqueue(&[named_song("c")], Placement::Last);
    player.enqueue(&[], Placement::Last);

    let told: Vec<_> = std::iter::from_fn(|| notices.try_recv().ok()).collect();
    assert_eq!(told, [PlayerNotice::Skipped; 3]);
}

#[test]
fn turning_on_skipping_takes_out_what_waits_but_not_what_plays() {
    let cx = &TestExecutor::new();
    let player = idle_player(cx);
    player.fill_queue(
        &[
            named_song("a"),
            named_song("b (Off Vocal)"),
            named_song("c [instrumental]"),
            named_song("d"),
            named_song("e (オリジナル・カラオケ)"),
        ],
        2,
    );
    player.pretend_playing(true);
    player.set_skip_instrumentals(false);
    assert_eq!(queued_titles(&player).len(), 5);

    player.set_skip_instrumentals(true);
    assert_eq!(queued_titles(&player), ["a", "c [instrumental]", "d"]);
    assert_eq!(current_title(&player).as_deref(), Some("c [instrumental]"));
}

#[test]
fn a_skipped_song_goes_when_the_player_moves_on_and_never_comes_back() {
    let cx = &TestExecutor::new();
    let player = idle_player(cx);
    player.fill_queue(
        &[
            named_song("a"),
            named_song("b [instrumental]"),
            named_song("c"),
        ],
        1,
    );
    player.set_repeat(RepeatMode::All);
    player.pretend_playing(true);
    player.set_skip_instrumentals(true);
    assert_eq!(queued_titles(&player).len(), 3);

    player.next();
    assert_eq!(current_title(&player).as_deref(), Some("c"));
    assert_eq!(queued_titles(&player), ["a", "c"]);

    player.next();
    assert_eq!(current_title(&player).as_deref(), Some("a"));
    assert_eq!(queued_titles(&player), ["a", "c"]);
}

#[test]
fn turning_off_skipping_keeps_the_current_song() {
    let cx = &TestExecutor::new();
    let player = idle_player(cx);
    player.set_skip_instrumentals(true);
    player.fill_queue(&[named_song("a [instrumental]"), named_song("b")], 0);
    player.pretend_playing(true);
    player.set_skip_instrumentals(true);
    player.set_skip_instrumentals(false);
    player.next();
    assert_eq!(queued_titles(&player), ["a [instrumental]", "b"]);
}

#[test]
fn play_and_pause_only_change_what_they_name() {
    let cx = &TestExecutor::new();
    let player = idle_player(cx);
    player.fill_queue(&[named_song("a")], 0);
    player.pretend_playing(true);
    let is_playing = || player.snapshot().is_playing();

    player.set_playing(true);
    assert!(is_playing());
    player.set_playing(false);
    assert!(!is_playing());
    player.set_playing(false);
    assert!(!is_playing());
    player.set_playing(true);
    assert!(is_playing());
    player.toggle_playback();
    assert!(!is_playing());
}

#[test]
fn stop_keeps_the_song_for_what_is_added_next() {
    let cx = &TestExecutor::new();
    let player = idle_player(cx);
    player.fill_queue(&[named_song("a"), named_song("b")], 0);
    player.pretend_playing(true);
    player.stop();
    assert!(!player.snapshot().is_playing());

    player.enqueue(&[named_song("c")], Placement::Last);
    assert_eq!(current_title(&player).as_deref(), Some("a"));
    assert!(!player.snapshot().is_playing());
}

#[test]
fn seeking_stays_in_the_song_until_its_end() {
    let cx = &TestExecutor::new();
    let player = idle_player(cx);
    player.fill_queue(&[named_song("a"), named_song("b")], 0);
    let seconds = Duration::from_secs;
    assert_eq!(player.seek_to(seconds(50)), None, "nothing has started");

    player.pretend_playing(true);
    assert_eq!(player.seek_to(seconds(50)), Some(seconds(50)));
    assert_eq!(player.seek_forward(), Some(seconds(10)));
    assert_eq!(player.seek_forward_by(seconds(20)), Some(seconds(20)));
    assert_eq!(player.seek_backward(), Some(Duration::ZERO));
    assert_eq!(current_title(&player).as_deref(), Some("a"));

    assert_eq!(player.seek_to(seconds(100)), None);
    assert_eq!(current_title(&player).as_deref(), Some("b"));
}

#[test]
fn opening_is_playing_unless_a_pause_came_meanwhile() {
    let cx = &TestExecutor::new();
    let player = idle_player(cx);
    player.fill_queue(&[named_song("a")], 0);
    assert!(player.snapshot().is_stopped());

    player.pretend_playing(true);
    assert!(player.snapshot().is_playing());
    player.pretend_playing(false);
    assert!(!player.snapshot().is_playing());

    player
        .0
        .core
        .update(|core| core.shown.state = State::Opening);
    assert!(player.snapshot().is_playing());
    player.0.core.update(|core| {
        core.shown.resume = Some(Resume {
            position: Duration::ZERO,
            paused: true,
        });
    });
    assert!(!player.snapshot().is_playing());
}

fn fail_song(player: &Player, error: audio::Error) {
    player
        .0
        .core
        .update(|core| core.song_failed(player, &error));
}

fn resume_of(player: &Player) -> Option<Resume> {
    read(player, |core| core.shown.resume)
}

#[test]
fn a_song_that_stops_arriving_picks_up_where_it_was() {
    let cx = &TestExecutor::new();
    let player = idle_player(cx);
    player.fill_queue(&[named_song("a")], 0);
    player.pretend_playing(true);

    fail_song(&player, audio::Error::Read("connection reset".to_owned()));
    assert_eq!(state(&player), State::Stopped);
    assert!(player.snapshot().error().is_some());
    assert_eq!(
        resume_of(&player),
        Some(Resume {
            position: Duration::ZERO,
            paused: false,
        }),
        "an idle engine is at zero; what matters is that the position is kept"
    );

    player.toggle_playback();
    assert_eq!(state(&player), State::Opening);
    assert!(resume_of(&player).is_some(), "Play did not start it over");
}

#[test]
fn a_song_that_cannot_be_decoded_starts_over() {
    let cx = &TestExecutor::new();
    let player = idle_player(cx);
    player.fill_queue(&[named_song("a")], 0);
    player.pretend_playing(true);

    fail_song(&player, audio::Error::Decode("bad frame".to_owned()));
    assert_eq!(state(&player), State::Stopped);
    assert_eq!(resume_of(&player), None);
}

#[test]
fn a_kept_file_that_breaks_is_fetched_again_from_where_it_was() {
    let cx = &TestExecutor::new();
    let player = idle_player(cx);
    player.fill_queue(&[named_song("a")], 0);
    player.pretend_playing(true);
    player.0.core.update(|core| core.loaded_kept = true);

    fail_song(&player, audio::Error::Decode("bad frame".to_owned()));
    assert_eq!(state(&player), State::Opening);
    assert!(resume_of(&player).is_some());
}

#[test]
fn another_output_picks_up_only_what_the_output_stopped() {
    let cx = &TestExecutor::new();
    let stopped_by = |error: audio::Error, cx: &TestExecutor| {
        let player = idle_player(cx);
        player.fill_queue(&[named_song("a")], 0);
        player.pretend_playing(true);
        fail_song(&player, error);
        assert_eq!(state(&player), State::Stopped);
        player.set_output(OutputSettings::default());
        player
    };

    let player = stopped_by(audio::Error::Output("gone".to_owned()), cx);
    assert_eq!(state(&player), State::Opening);

    let player = stopped_by(audio::Error::Read("connection reset".to_owned()), cx);
    assert_eq!(state(&player), State::Stopped);
    assert!(player.snapshot().error().is_some());

    // A pause while the song opened leaves a position to pick up at, and then it fails to open
    let player = idle_player(cx);
    player.fill_queue(&[named_song("a")], 0);
    player.0.core.update(|core| {
        core.shown.state = State::Opening;
        core.shown.resume = Some(Resume {
            position: Duration::ZERO,
            paused: true,
        });
        let song = core.shown.queue.current().cloned().unwrap();
        core.fail(&song, "The server didn't answer.");
    });
    player.set_output(OutputSettings::default());
    assert_eq!(state(&player), State::Stopped);
    assert!(player.snapshot().error().is_some());
}

#[test]
fn a_fault_in_the_engine_stops_the_player_and_says_so() {
    let cx = &TestExecutor::new();
    // The engine's thread wakes the test as it ends
    cx.allow_parking();
    let player = idle_player(cx);
    player.fill_queue(&[named_song("a")], 0);
    player.pretend_playing(true);

    read(&player, |core| core.engine.crash());
    wait_until(&player, cx, "the fault to be told", |core| {
        core.shown.state == State::Stopped && core.shown.error.is_some()
    });

    player.toggle_playback();
    assert_eq!(state(&player), State::Stopped);
    let error = player.snapshot().error().unwrap().to_owned();
    assert!(error.ends_with(ENGINE_GONE), "{error}");
}

#[test]
fn a_fault_in_the_engine_with_nothing_queued_is_told_too() {
    let cx = &TestExecutor::new();
    cx.allow_parking();
    let player = idle_player(cx);

    read(&player, |core| core.engine.crash());
    wait_until(&player, cx, "the fault to be told", |core| {
        core.shown.error.is_some()
    });

    assert_eq!(player.snapshot().error(), Some(ENGINE_GONE));
}

#[test]
fn shutting_down_is_not_told_as_a_fault() {
    let cx = &TestExecutor::new();
    let player = idle_player(cx);
    player.fill_queue(&[named_song("a")], 0);
    player.pretend_playing(true);

    player.shutdown();
    cx.run_until_parked();

    assert_eq!(player.snapshot().error(), None);
}

#[test]
fn every_change_is_published_at_once() {
    let cx = &TestExecutor::new();
    let player = idle_player(cx);
    let mut changes = player.subscribe();
    player.fill_queue(&[named_song("a"), named_song("b")], 0);
    assert!(changes.has_changed().unwrap());
    assert_eq!(
        changes
            .borrow_and_update()
            .current_song()
            .map(|song| song.title().to_owned())
            .as_deref(),
        Some("a")
    );

    player.cycle_repeat();
    assert_eq!(changes.borrow_and_update().repeat(), RepeatMode::All);
    player.toggle_shuffle();
    assert!(changes.borrow_and_update().is_shuffled());
    player.set_repeat(RepeatMode::All);
    assert!(!changes.has_changed().unwrap(), "nothing changed");
}

fn player_saving_to(store: &config::Store, cx: &TestExecutor) -> Player {
    let executor = cx.executor();
    Player::new(
        executor,
        session("http://127.0.0.1:9"),
        Ok(store.clone()),
        OutputSettings::default(),
        None,
    )
}

#[test]
fn a_volume_step_is_kept_for_the_next_launch() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let player = player_saving_to(&store, cx);
    assert_eq!(player.snapshot().volume(), 1.0);

    player.volume_down();
    player.volume_down();
    cx.run_until_parked();
    assert_eq!(store.volume().unwrap(), Some(0.8));
    assert_eq!(player_saving_to(&store, cx).snapshot().volume(), 0.8);
}

#[test]
fn the_saved_volume_is_used_though_another_setting_cannot_be_read() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    std::fs::write(
        store.config_path(),
        "volume = 0.2\ncrossfade_seconds = -1\n",
    )
    .unwrap();

    let player = player_saving_to(&store, cx);

    assert_eq!(player.snapshot().volume(), 0.2);
}

#[test]
fn a_dragged_volume_is_kept_only_once_let_go() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let player = player_saving_to(&store, cx);

    player.set_volume(0.3);
    cx.run_until_parked();
    assert_eq!(store.volume().unwrap(), None);
    player.save_volume();
    cx.run_until_parked();
    assert_eq!(store.volume().unwrap(), Some(0.3));
}

#[test]
fn muting_keeps_the_level_and_moving_it_unmutes() {
    let cx = &TestExecutor::new();
    let player = idle_player(cx);
    player.set_volume(0.5);
    player.toggle_mute();
    assert!(player.snapshot().is_muted());
    assert_eq!(player.snapshot().volume(), 0.5);

    player.set_volume(0.4);
    assert!(!player.snapshot().is_muted());
    player.toggle_mute();
    player.set_volume(0.0);
    assert!(player.snapshot().is_muted());
}

#[test]
fn unmuting_at_no_volume_brings_back_the_level_kept_before() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let player = player_saving_to(&store, cx);
    player.set_volume(0.7);
    player.save_volume();
    player.set_volume(0.2);
    player.set_volume(0.0);
    player.save_volume();
    assert!(player.snapshot().is_silent());

    player.toggle_mute();
    assert!(!player.snapshot().is_silent());
    assert_eq!(player.snapshot().volume(), 0.7);
    cx.run_until_parked();
    assert_eq!(store.volume().unwrap(), Some(0.7));

    player.toggle_mute();
    assert!(player.snapshot().is_muted());
    player.set_volume(0.0);
    player.toggle_mute();
    assert!(!player.snapshot().is_muted());
    assert_eq!(player.snapshot().volume(), 0.7);
}

#[test]
fn unmuting_at_no_volume_with_no_level_known_goes_halfway() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    store.save_volume(0.0).unwrap();
    let player = player_saving_to(&store, cx);
    assert!(player.snapshot().is_silent());

    player.toggle_mute();
    assert_eq!(player.snapshot().volume(), 0.5);
    assert!(!player.snapshot().is_muted());
}

#[test]
fn only_a_started_song_with_a_duration_is_seekable() {
    let cx = &TestExecutor::new();
    let player = idle_player(cx);
    let untimed: Song = serde_json::from_value(json!({ "id": "u", "title": "u" })).unwrap();
    player.fill_queue(&[named_song("a")], 0);
    assert!(!player.snapshot().is_seekable());
    player.pretend_playing(false);
    assert!(player.snapshot().is_seekable());
    player.fill_queue(&[untimed], 0);
    assert!(!player.snapshot().is_seekable());
}

#[test]
fn setting_the_repeat_changes_what_follows() {
    let cx = &TestExecutor::new();
    let player = idle_player(cx);
    player.fill_queue(&[named_song("a"), named_song("b")], 0);
    player.pretend_playing(true);
    player.set_skip_instrumentals(false);
    let following = |player: &Player| {
        read(player, |core| {
            let prepared = core.following.as_ref().map(|following| following.entry);
            (prepared, core.shown.queue.following_id())
        })
    };
    let (prepared, due) = following(&player);
    assert!(prepared.is_some());
    assert_eq!(prepared, due);

    for repeat in [RepeatMode::One, RepeatMode::Off, RepeatMode::All] {
        player.set_repeat(repeat);
        let (prepared, due) = following(&player);
        assert_eq!(prepared, due, "after repeating {repeat:?}");
    }
}

#[test]
fn gain_follows_the_square_of_the_level() {
    assert_eq!(gain(1.0, false), 1.0);
    assert_eq!(gain(0.5, false), 0.25);
    assert_eq!(gain(1.0, true), 0.0);
}

#[test]
fn a_held_device_is_told_apart_from_another_failure() {
    assert!(describe(&audio::Error::DeviceHeld).contains("Another app"));
    assert!(!describe(&audio::Error::ExclusiveOutput(String::new())).contains("Another app"));
    assert!(describe(&audio::Error::ExclusiveNotAllowed).contains("Exclusive mode"));
}

/// A silent 16-bit stereo WAV.
fn wav(millis: u32) -> Vec<u8> {
    let frames = RATE * millis / 1_000;
    let data_len = frames * 4;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&RATE.to_le_bytes());
    bytes.extend_from_slice(&(RATE * 4).to_le_bytes());
    bytes.extend_from_slice(&4u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    bytes.resize(44 + data_len as usize, 0);
    bytes
}

/// Serves songs by ID and notes which were asked for, in order.
#[derive(Clone, Default)]
struct Server {
    songs: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    streamed: Arc<Mutex<Vec<String>>>,
    /// While set, the body of the song named keeps its last half back.
    held: Arc<Mutex<Option<String>>>,
    released: Arc<AtomicBool>,
    /// The connection is lost after this many bytes of the song named.
    cut: Arc<Mutex<Option<(String, usize)>>>,
}

impl Server {
    fn start(self) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}/", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for socket in listener.incoming() {
                let Ok(socket) = socket else { return };
                let server = self.clone();
                std::thread::spawn(move || server.answer(socket));
            }
        });
        address
    }

    fn answer(&self, mut socket: TcpStream) {
        let mut reader = BufReader::new(socket.try_clone().unwrap());
        let mut request = String::new();
        reader.read_line(&mut request).ok();
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap_or(0) > 2 {
            line.clear();
        }
        let streamed = request.contains("/stream");
        let id = request
            .split(['?', '&', ' '])
            .find_map(|part| part.strip_prefix("id="))
            .unwrap_or_default()
            .to_owned();
        let song = streamed
            .then(|| {
                self.streamed.lock().unwrap().push(id.clone());
                self.songs.lock().unwrap().get(&id).cloned()
            })
            .flatten();
        let Some(body) = song else {
            let status = if streamed { "404 Not Found" } else { "200 OK" };
            let reply =
                format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            socket.write_all(reply.as_bytes()).ok();
            return;
        };
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: audio/wav\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        socket.write_all(headers.as_bytes()).ok();
        if let Some((_, len)) = self.cut.lock().unwrap().as_ref().filter(|cut| cut.0 == id) {
            socket.write_all(&body[..*len]).ok();
            return;
        }
        let held = self.held.lock().unwrap().as_deref() == Some(id.as_str());
        let head = if held { body.len() / 2 } else { body.len() };
        socket.write_all(&body[..head]).ok();
        while !self.released.load(Ordering::SeqCst) && head < body.len() {
            std::thread::sleep(Duration::from_millis(5));
        }
        socket.write_all(&body[head..]).ok();
        // Until the client has gone, so that it decides when a body ends
        let _ = reader.read(&mut [0; 1]);
    }

    fn add(&self, id: &str, body: Vec<u8>) -> Song {
        let song = json!({
            "id": id, "title": id, "suffix": "wav", "size": body.len(), "duration": 1
        });
        self.songs.lock().unwrap().insert(id.to_owned(), body);
        serde_json::from_value(song).unwrap()
    }

    fn streamed(&self) -> Vec<String> {
        self.streamed.lock().unwrap().clone()
    }
}

/// A player whose engine ends with the test, also one that fails. An engine left behind goes on
/// playing, and reopens and restores rates on the device under later tests, until no stream
/// there gets called any more.
struct TestPlayer(Player);

impl std::ops::Deref for TestPlayer {
    type Target = Player;

    fn deref(&self) -> &Player {
        &self.0
    }
}

impl Drop for TestPlayer {
    fn drop(&mut self) {
        self.0.shutdown();
    }
}

fn player(server: &Server, cx: &TestExecutor) -> TestPlayer {
    // The engine and the network wake the test from their own threads
    cx.allow_parking();
    let output = OutputSettings::new(Some(player_device()), None);
    let address = server.clone().start();
    let executor = cx.executor();
    TestPlayer(Player::new(
        executor,
        session(&address),
        Err(config::Error::NoHomeDir),
        output,
        None,
    ))
}

fn wait_until(
    player: &Player,
    cx: &TestExecutor,
    what: &str,
    mut condition: impl FnMut(&Core) -> bool,
) {
    let started = Instant::now();
    loop {
        cx.run_until_parked();
        if player.0.core.read(&mut condition) {
            return;
        }
        assert!(started.elapsed() < WAIT, "still waiting: {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn read<R>(player: &Player, read: impl FnOnce(&Core) -> R) -> R {
    player.0.core.read(read)
}

fn state(player: &Player) -> State {
    player.0.core.read(|core| core.shown.state)
}

fn current(core: &Core) -> Option<&str> {
    core.shown.queue.current().map(|song| song.id().as_str())
}

fn is_playing_song(core: &Core, id: &str) -> bool {
    core.shown.state == State::Playing && current(core) == Some(id)
}

fn dop_output() -> OutputSettings {
    OutputSettings::new(Some(player_device()), None).with_dop(Some(audio::DsdSettings::default()))
}

fn player_device() -> String {
    let name =
        std::env::var("HIPHILE_TEST_DEVICE").expect("name a virtual device in HIPHILE_TEST_DEVICE");
    audio::output_devices()
        .unwrap()
        .into_iter()
        .find(|device| device.name().trim() == name)
        .and_then(|device| device.id().map(str::to_owned))
        .expect("no such device")
}

#[test]
#[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
fn turning_dop_on_pauses_a_playing_song_and_locks_the_volume() {
    let cx = &TestExecutor::new();
    let server = Server::default();
    let songs = [server.add("a", wav(5_000))];
    let player = player(&server, cx);
    player.play(&songs, 0);
    wait_until(&player, cx, "a to play", |core| is_playing_song(core, "a"));

    player.set_output(dop_output());
    assert_eq!(state(&player), State::Paused);
    assert!(player.snapshot().is_volume_locked());

    player.set_output(OutputSettings::new(Some(player_device()), None));
    assert!(!player.snapshot().is_volume_locked());
}

#[test]
#[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
fn after_turning_dop_on_songs_wait_for_play() {
    let cx = &TestExecutor::new();
    let server = Server::default();
    let songs = [server.add("a", wav(5_000)), server.add("b", wav(5_000))];
    let player = player(&server, cx);
    player.set_output(dop_output());
    player.play(&songs, 0);
    assert!(!player.snapshot().is_playing());
    wait_until(&player, cx, "a to wait", |core| {
        current(core) == Some("a") && core.shown.state == State::Paused
    });

    player.next();
    wait_until(&player, cx, "b to wait", |core| {
        current(core) == Some("b") && core.shown.state == State::Paused
    });

    player.toggle_playback();
    assert_eq!(state(&player), State::Playing);
    assert_eq!(read(&player, |core| core.dop_start), DopStart::Settled);

    player.previous();
    wait_until(&player, cx, "a to play", |core| is_playing_song(core, "a"));
}

#[test]
#[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
fn turning_dop_on_while_a_song_opens_makes_it_wait() {
    let cx = &TestExecutor::new();
    let server = Server::default();
    let songs = [server.add("a", wav(5_000))];
    let player = player(&server, cx);
    player.play(&songs, 0);
    player.set_output(dop_output());
    wait_until(&player, cx, "a to wait", |core| {
        current(core) == Some("a") && core.shown.state == State::Paused
    });
}

#[test]
#[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
fn play_goes_straight_on_with_nothing_open_after_turning_dop_on() {
    let cx = &TestExecutor::new();
    let server = Server::default();
    let songs = [server.add("a", wav(5_000))];
    let player = player(&server, cx);
    player.fill_queue(&songs, 0);
    player.set_output(dop_output());
    player.toggle_playback();
    wait_until(&player, cx, "a to play", |core| is_playing_song(core, "a"));
    assert_eq!(read(&player, |core| core.dop_start), DopStart::Settled);
}

#[test]
#[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
fn repeating_one_song_plays_it_until_repeat_is_off() {
    let cx = &TestExecutor::new();
    let server = Server::default();
    let songs = [server.add("a", wav(300)), server.add("b", wav(300))];
    let player = player(&server, cx);
    player.set_repeat(RepeatMode::One);
    player.play(&songs, 0);
    wait_until(&player, cx, "a to come round again", |_| {
        server.streamed().iter().filter(|id| *id == "a").count() >= 3
    });
    read(&player, |core| assert_eq!(current(core), Some("a")));
    assert!(!server.streamed().contains(&"b".to_owned()));

    player.set_repeat(RepeatMode::Off);
    wait_until(&player, cx, "the end", |core| {
        core.shown.state == State::Stopped
    });
    read(&player, |core| {
        assert_eq!(current(core), None);
        assert!(core.shown.error.is_none());
    });
}

#[test]
#[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
fn a_pause_while_a_song_opens_holds() {
    let cx = &TestExecutor::new();
    let server = Server::default();
    let songs = [server.add("a", wav(5_000))];
    let player = player(&server, cx);
    player.play(&songs, 0);
    assert_eq!(state(&player), State::Opening);
    assert!(player.snapshot().is_playing());
    player.toggle_playback();
    assert_eq!(state(&player), State::Opening);
    assert!(!player.snapshot().is_playing());
    wait_until(&player, cx, "the song to start", |core| {
        core.shown.is_started()
    });
    assert_eq!(state(&player), State::Paused);
    player.toggle_playback();
    assert_eq!(state(&player), State::Playing);
}

#[test]
#[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
fn a_pause_does_not_outlast_a_song_that_failed_to_open() {
    let cx = &TestExecutor::new();
    let server = Server::default();
    let song = server.add("a", wav(1_000));
    let body = server.songs.lock().unwrap().remove("a").unwrap();
    let player = player(&server, cx);
    player.play(std::slice::from_ref(&song), 0);
    player.toggle_playback();
    wait_until(&player, cx, "the failure", |core| {
        core.shown.error.is_some()
    });

    server.songs.lock().unwrap().insert("a".to_owned(), body);
    player.toggle_playback();
    wait_until(&player, cx, "the song to start", |core| {
        core.shown.is_started()
    });
    assert_eq!(state(&player), State::Playing);
}

#[test]
#[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
fn a_song_waiting_for_its_data_answers_the_controls() {
    let cx = &TestExecutor::new();
    let server = Server::default();
    // Half of it arrives, which sounds before it runs out
    let songs = [server.add("a", wav(6_000))];
    *server.held.lock().unwrap() = Some("a".to_owned());
    let player = player(&server, cx);
    player.play(&songs, 0);
    wait_until(&player, cx, "the song to wait", |core| {
        core.shown.is_started() && core.shown.is_buffering()
    });

    player.toggle_playback();
    assert_eq!(state(&player), State::Paused);
    assert!(!player.snapshot().is_buffering());
    wait_until(&player, cx, "the wait to stop counting", |core| {
        !core.shown.buffering
    });
    player.toggle_playback();
    assert_eq!(state(&player), State::Playing);
    wait_until(&player, cx, "the song to wait again", |core| {
        core.shown.is_buffering()
    });

    server.released.store(true, Ordering::SeqCst);
    wait_until(&player, cx, "the song to sound", |core| {
        !core.shown.is_buffering()
    });
}

fn is_following_set(core: &Core, id: &str) -> bool {
    core.shown.queue.following().map(|song| song.id().as_str()) == Some(id)
        && core
            .following
            .as_ref()
            .is_some_and(|following| following.load.is_some())
}

#[test]
#[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
fn the_next_song_is_fetched_once_the_current_one_has_arrived() {
    let cx = &TestExecutor::new();
    let server = Server::default();
    let songs = [
        server.add("a", wav(2_000)),
        server.add("b", wav(500)),
        server.add("c", wav(300)),
    ];
    *server.held.lock().unwrap() = Some("a".to_owned());
    let player = player(&server, cx);
    player.play(&songs, 0);

    wait_until(&player, cx, "a to play", |core| is_playing_song(core, "a"));
    std::thread::sleep(Duration::from_millis(100));
    cx.run_until_parked();
    assert_eq!(server.streamed(), ["a"]);

    server.released.store(true, Ordering::SeqCst);
    wait_until(&player, cx, "b to be set", |core| {
        is_following_set(core, "b")
    });
    assert_eq!(server.streamed(), ["a", "b"]);

    wait_until(&player, cx, "b to play", |core| is_playing_song(core, "b"));
    assert_eq!(
        player.snapshot().duration(),
        Some(Duration::from_millis(500))
    );
    wait_until(&player, cx, "c to play", |core| is_playing_song(core, "c"));
    wait_until(&player, cx, "the end", |core| {
        core.shown.state == State::Stopped
    });
    assert_eq!(server.streamed(), ["a", "b", "c"]);
    read(&player, |core| {
        assert_eq!(current(core), None);
        assert!(core.following.is_none());
        assert!(core.arrival.is_none());
        assert!(core.shown.error.is_none());
    });
}

#[test]
#[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
fn moving_another_song_next_makes_it_follow() {
    let cx = &TestExecutor::new();
    let server = Server::default();
    let songs = [
        server.add("a", wav(1_500)),
        server.add("b", wav(5_000)),
        server.add("c", wav(5_000)),
    ];
    let player = player(&server, cx);
    player.play(&songs, 0);
    wait_until(&player, cx, "b to be set", |core| {
        is_following_set(core, "b")
    });

    let last = player.snapshot().queue().entries()[2].id();
    player.move_entry_to_next(last);
    wait_until(&player, cx, "c to be set", |core| {
        is_following_set(core, "c")
    });
    wait_until(&player, cx, "c to play", |core| is_playing_song(core, "c"));
    assert_eq!(server.streamed()[..3], ["a", "b", "c"]);
}

#[test]
#[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
fn a_crossfade_shows_the_next_song_as_it_starts_fading_in() {
    let cx = &TestExecutor::new();
    let server = Server::default();
    let songs = [server.add("a", wav(2_000)), server.add("b", wav(5_000))];
    let player = player(&server, cx);
    player.set_transitions(Transitions {
        trim_silence: false,
        crossfade: Duration::from_secs(1),
        fades_play_and_pause: false,
    });
    player.play(&songs, 0);
    wait_until(&player, cx, "a to play", |core| is_playing_song(core, "a"));
    let started = Instant::now();
    wait_until(&player, cx, "b to play", |core| is_playing_song(core, "b"));
    let played = started.elapsed();
    assert!(played < Duration::from_millis(1_500), "{played:?}");
    assert!(played > Duration::from_millis(500), "{played:?}");
}

/// With nothing left to come next, playback ends after the song, even when
/// the removed song was already on its way to the device. That its audio
/// is taken back is for the engine's tests, which can hear it.
#[test]
#[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
fn removing_the_next_song_ends_playback_after_the_current_one() {
    let cx = &TestExecutor::new();
    let server = Server::default();
    let songs = [server.add("a", wav(1_000)), server.add("b", wav(5_000))];
    let player = player(&server, cx);
    player.play(&songs, 0);
    wait_until(&player, cx, "b to be set", |core| {
        is_following_set(core, "b")
    });
    // Within what the engine writes ahead of the end of the song
    wait_until(&player, cx, "the end of a to be near", |core| {
        core.engine.position() > Duration::from_millis(700)
    });
    let next = player.snapshot().queue().entries()[1].id();
    player.remove_entry(next);
    wait_until(&player, cx, "the end", |core| {
        core.shown.state == State::Stopped
    });
    assert!(player.snapshot().error().is_none());
}

#[test]
#[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
fn repeating_all_songs_goes_back_to_the_first() {
    let cx = &TestExecutor::new();
    let server = Server::default();
    let songs = [server.add("a", wav(1_500)), server.add("b", wav(1_500))];
    let player = player(&server, cx);
    player.play(&songs, 1);
    wait_until(&player, cx, "b to play", |core| is_playing_song(core, "b"));

    player.set_repeat(RepeatMode::All);
    wait_until(&player, cx, "a to be set", |core| {
        is_following_set(core, "a")
    });
    wait_until(&player, cx, "a to play", |core| is_playing_song(core, "a"));
}

#[test]
#[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
fn shuffling_while_a_song_plays_changes_what_follows_it() {
    let cx = &TestExecutor::new();
    let server = Server::default();
    let songs: Vec<Song> = (0..30)
        .map(|ix| server.add(&format!("s{ix}"), wav(1_000)))
        .collect();
    let player = player(&server, cx);
    player.play(&songs, 0);
    wait_until(&player, cx, "s1 to be set", |core| {
        is_following_set(core, "s1")
    });

    let following = || {
        read(&player, |core| {
            core.shown
                .queue
                .following()
                .map(|song| song.id().as_str().to_owned())
        })
    };
    // One time in 29, the shuffle leaves s1 next
    let next = loop {
        player.toggle_shuffle();
        let next = following().unwrap();
        if next != "s1" {
            break next;
        }
        player.toggle_shuffle();
    };
    wait_until(&player, cx, "the shuffled song to be set", |core| {
        is_following_set(core, &next)
    });
    wait_until(&player, cx, "the shuffled song to play", |core| {
        is_playing_song(core, &next)
    });
}

#[test]
#[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
fn a_next_song_that_cannot_be_fetched_fails_at_its_turn() {
    let cx = &TestExecutor::new();
    let server = Server::default();
    let songs = [
        server.add("a", wav(1_000)),
        serde_json::from_value(json!({ "id": "gone", "title": "Gone" })).unwrap(),
    ];
    let player = player(&server, cx);
    player.play(&songs, 0);
    wait_until(&player, cx, "the attempt", |_| {
        server.streamed() == ["a", "gone"]
    });
    cx.run_until_parked();
    read(&player, |core| {
        assert!(is_playing_song(core, "a"));
        assert!(core.shown.error.is_none());
    });
    wait_until(&player, cx, "the failure", |core| {
        core.shown.error.is_some()
    });
    read(&player, |core| {
        assert_eq!(current(core), Some("gone"));
        assert_eq!(core.shown.state, State::Stopped);
    });
    assert_eq!(server.streamed(), ["a", "gone", "gone"]);
}

#[test]
#[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
fn a_next_song_that_cannot_open_is_tried_once_more() {
    let cx = &TestExecutor::new();
    let server = Server::default();
    let songs = [
        server.add("a", wav(1_000)),
        server.add("broken", b"not audio".to_vec()),
    ];
    let player = player(&server, cx);
    player.play(&songs, 0);
    wait_until(&player, cx, "the failure", |core| {
        core.shown.error.is_some()
    });
    read(&player, |core| {
        assert_eq!(current(core), Some("broken"));
        assert_eq!(core.shown.state, State::Stopped);
        assert!(core.arrival.is_none());
    });
    assert_eq!(server.streamed(), ["a", "broken", "broken"]);
}

#[test]
#[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
fn a_next_song_that_breaks_as_it_joins_is_tried_once_more() {
    let cx = &TestExecutor::new();
    let server = Server::default();
    let songs = [server.add("a", wav(1_000)), server.add("b", wav(1_000))];
    *server.cut.lock().unwrap() = Some(("b".to_owned(), 44 + 16));
    let player = player(&server, cx);
    player.play(&songs, 0);
    wait_until(&player, cx, "the failure", |core| {
        core.shown.error.is_some()
    });
    read(&player, |core| {
        assert_eq!(current(core), Some("b"));
        assert_eq!(core.shown.state, State::Stopped);
    });
    let tries = server.streamed();
    assert_eq!(tries.iter().filter(|id| *id == "b").count(), 2, "{tries:?}");
}

/// A pause that crosses the events of the next song starting stays: the
/// engine has paused the song, so the player must not show it as playing.
#[test]
#[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
fn a_pause_as_the_next_song_starts_holds() {
    let cx = &TestExecutor::new();
    let server = Server::default();
    let songs = [server.add("a", wav(1_000)), server.add("b", wav(5_000))];
    let player = player(&server, cx);
    player.play(&songs, 0);
    wait_until(&player, cx, "b to be set", |core| {
        is_following_set(core, "b")
    });

    // Without running the player, so that the events wait for it
    let position = || read(&player, |core| core.engine.position());
    let started = Instant::now();
    let mut last = position();
    while position() >= last {
        last = position();
        assert!(started.elapsed() < WAIT, "still waiting for the next song");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(read(&player, |core| is_playing_song(core, "a")));
    player.toggle_playback();

    wait_until(&player, cx, "b to start", |core| {
        current(core) == Some("b") && core.shown.format.is_some()
    });
    assert_eq!(state(&player), State::Paused);
    std::thread::sleep(Duration::from_millis(100));
    let paused_at = position();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(position(), paused_at);

    player.toggle_playback();
    assert_eq!(state(&player), State::Playing);
    std::thread::sleep(Duration::from_millis(100));
    assert!(position() > paused_at);
}
