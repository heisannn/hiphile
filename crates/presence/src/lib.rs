//! Shows the song playing as the user's Discord status, as "Listening to".
//!
//! Discord may be missing, may start or quit at any time, and may be slow to
//! answer, and playback depends on none of it. So a thread of its own talks to
//! Discord, and failures are only retried.

mod imgbb;

pub use catalog::CoverLink;
pub use imgbb::{CoverHost, Error};

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use discord_rich_presence::activity::{Activity, ActivityType, Assets, Timestamps};
use discord_rich_presence::{DiscordIpc, DiscordIpcClient};
use subsonic::{CoverArtId, Song, SongId};

const CLIENT_ID: &str = "1567455781450678443";
/// Discord limits how fast a status may change, so skipping through songs
/// sends only where it ended up.
const MIN_GAP: Duration = Duration::from_secs(4);
/// How long a change waits for others, so that a cover already known goes with
/// its song rather than a [`MIN_GAP`] after it.
const SETTLE: Duration = Duration::from_millis(300);
/// How often the status is sent again, which is how a Discord that started or
/// restarted since gets it.
const KEEP_ALIVE: Duration = Duration::from_secs(15);
/// A start time that moved less than this is the position's usual jitter
/// rather than a seek.
const LONGEST_DRIFT: Duration = Duration::from_secs(2);
const OPCODE_CLOSE: u32 = 2;
/// Discord rejects a whole status over one text outside these lengths, counted
/// in UTF-16 units.
const TEXT_UNITS: std::ops::RangeInclusive<usize> = 2..=128;

/// Connects once there is something to show, and dropping it takes the status away.
pub struct Presence {
    updates: Sender<Option<Listening>>,
    /// While off, Discord hears nothing and is not connected to for it.
    is_on: bool,
    shown: Option<Shown>,
    covers: HashMap<CoverArtId, CoverLink>,
}

struct Shown {
    song: SongId,
    cover: Option<CoverArtId>,
    started: SystemTime,
    listening: Listening,
}

impl Presence {
    pub fn spawn() -> Self {
        let (updates, received) = mpsc::channel();
        let connect = || {
            let mut client = DiscordIpcClient::new(CLIENT_ID);
            client.connect().ok().map(|()| client)
        };
        // Without the thread the updates go nowhere, and there is no status
        thread::Builder::new()
            .name("discord-presence".into())
            .spawn(move || run(received, connect, TIMING))
            .ok();
        Self::sending_to(updates)
    }

    pub fn disconnected() -> Self {
        let (updates, _) = mpsc::channel();
        Self::sending_to(updates)
    }

    fn sending_to(updates: Sender<Option<Listening>>) -> Self {
        Self {
            updates,
            is_on: true,
            shown: None,
            covers: HashMap::new(),
        }
    }

    pub fn is_on(&self) -> bool {
        self.is_on
    }

    /// Turning it on shows nothing until the next song or refresh.
    pub fn set_on(&mut self, is_on: bool) {
        self.is_on = is_on;
        if !is_on {
            self.clear();
        }
    }

    /// Cheap enough to call on every refresh: Discord hears of a new song or a seek only.
    pub fn listening(&mut self, song: &Song, position: Duration, duration: Option<Duration>) {
        if !self.is_on {
            return;
        }
        let now = SystemTime::now();
        let started = now.checked_sub(position).unwrap_or(now);
        let unchanged = self.shown.as_ref().is_some_and(|shown| {
            let drift = shown
                .started
                .duration_since(started)
                .or_else(|_| started.duration_since(shown.started))
                .unwrap_or_default();
            shown.song == *song.id() && drift < LONGEST_DRIFT
        });
        if unchanged {
            return;
        }
        let mut listening = Listening::new(song, started, duration);
        listening.cover = song
            .cover_art()
            .and_then(|cover| self.live_cover(cover))
            .map(|link| link.url.clone());
        self.updates.send(Some(listening.clone())).ok();
        self.shown = Some(Shown {
            song: song.id().clone(),
            cover: song.cover_art().cloned(),
            started,
            listening,
        });
    }

    /// Whether `cover` has a public URL that still works.
    pub fn has_cover(&self, cover: &CoverArtId) -> bool {
        self.live_cover(cover).is_some()
    }

    fn live_cover(&self, cover: &CoverArtId) -> Option<&CoverLink> {
        self.covers
            .get(cover)
            .filter(|link| link.is_live(SystemTime::now()))
    }

    pub fn set_cover(&mut self, cover: CoverArtId, link: CoverLink) {
        if let Some(shown) = &mut self.shown
            && shown.cover.as_ref() == Some(&cover)
        {
            shown.listening.cover = Some(link.url.clone());
            self.updates.send(Some(shown.listening.clone())).ok();
        }
        self.covers.insert(cover, link);
    }

    pub fn clear(&mut self) {
        if self.shown.take().is_some() {
            self.updates.send(None).ok();
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Listening {
    title: Option<String>,
    /// The artist and album, as the player's bar shows them.
    line: Option<String>,
    /// Milliseconds since the Unix epoch.
    started: i64,
    /// Likewise, if the song's length is known. With it, Discord shows a bar.
    ends: Option<i64>,
    cover: Option<String>,
}

impl Listening {
    fn new(song: &Song, started: SystemTime, duration: Option<Duration>) -> Self {
        let started = unix_millis(started);
        Self {
            title: fit(song.title()),
            line: match (song.artist(), song.album()) {
                (Some(artist), Some(album)) => fit(&format!("{artist} — {album}")),
                (Some(line), None) | (None, Some(line)) => fit(line),
                (None, None) => None,
            },
            started,
            ends: duration.map(|duration| started.saturating_add(duration.as_millis() as i64)),
            cover: None,
        }
    }

    fn activity(&self) -> Activity<'_> {
        let mut timestamps = Timestamps::new().start(self.started);
        if let Some(ends) = self.ends {
            timestamps = timestamps.end(ends);
        }
        let mut activity = Activity::new()
            .activity_type(ActivityType::Listening)
            .timestamps(timestamps);
        if let Some(title) = &self.title {
            activity = activity.details(title.as_str());
        }
        if let Some(line) = &self.line {
            activity = activity.state(line.as_str());
        }
        if let Some(cover) = &self.cover {
            activity = activity.assets(Assets::new().large_image(cover.as_str()));
        }
        activity
    }
}

/// Makes `text` a length Discord accepts: a song called "X" gets an invisible
/// character, and a long title is cut short. `None` for no text at all, which
/// Discord accepts only if it is left out.
fn fit(text: &str) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    let units = text.encode_utf16().count();
    if units < *TEXT_UNITS.start() {
        return Some(format!("{text}\u{200b}"));
    }
    if units <= *TEXT_UNITS.end() {
        return Some(text.to_owned());
    }
    let mut room = *TEXT_UNITS.end() - '…'.len_utf16();
    let mut fitted: String = text
        .chars()
        .take_while(|c| {
            let fits = c.len_utf16() <= room;
            room = room.saturating_sub(c.len_utf16());
            fits
        })
        .collect();
    fitted.push('…');
    Some(fitted)
}

fn unix_millis(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as i64)
}

#[derive(Clone, Copy)]
struct Timing {
    min_gap: Duration,
    settle: Duration,
    keep_alive: Duration,
}

const TIMING: Timing = Timing {
    min_gap: MIN_GAP,
    settle: SETTLE,
    keep_alive: KEEP_ALIVE,
};

/// Delivers what `updates` last asked for until the sender is dropped: a change
/// after `settle`, at most one per `min_gap`, and the same again every
/// `keep_alive`.
fn run<C: DiscordIpc>(
    updates: Receiver<Option<Listening>>,
    mut connect: impl FnMut() -> Option<C>,
    timing: Timing,
) {
    let mut connection = None;
    let mut wanted = None;
    let mut changed = false;
    let mut next_allowed = Instant::now();
    let mut due = next_allowed + timing.keep_alive;
    loop {
        match updates.recv_timeout(due.saturating_duration_since(Instant::now())) {
            Ok(update) => {
                wanted = update;
                // Only the first of several, so that they cannot put it off
                if !changed {
                    changed = true;
                    due = next_allowed.max(Instant::now() + timing.settle);
                }
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {}
        }
        deliver(&mut connection, wanted.as_ref(), &mut connect);
        let now = Instant::now();
        // Sending the same status again must not hold back a new one
        if changed {
            next_allowed = now + timing.min_gap;
            changed = false;
        }
        due = now + timing.keep_alive;
    }
}

/// Sends the status, or clears it without connecting just to do so. A
/// connection that fails is replaced once, as it may be from before Discord
/// restarted.
fn deliver<C: DiscordIpc>(
    connection: &mut Option<C>,
    wanted: Option<&Listening>,
    connect: &mut impl FnMut() -> Option<C>,
) {
    let Some(listening) = wanted else {
        if let Some(open) = connection
            && !cleared(open)
        {
            *connection = None;
        }
        return;
    };
    for _ in 0..2 {
        if connection.is_none() {
            *connection = connect();
        }
        let Some(open) = connection else {
            return;
        };
        if sent(open, listening) {
            return;
        }
        *connection = None;
    }
}

fn sent<C: DiscordIpc>(connection: &mut C, listening: &Listening) -> bool {
    connection.set_activity(listening.activity()).is_ok() && answered(connection)
}

fn cleared<C: DiscordIpc>(connection: &mut C) -> bool {
    connection.clear_activity().is_ok() && answered(connection)
}

/// Reads Discord's answer, which would otherwise pile up in the socket.
fn answered<C: DiscordIpc>(connection: &mut C) -> bool {
    connection
        .recv()
        .is_ok_and(|(opcode, _)| opcode != OPCODE_CLOSE)
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use discord_rich_presence::error::Error;
    use serde_json::{Value, json};

    use super::*;

    const SHORT: Duration = Duration::from_millis(5);
    const NEVER: Duration = Duration::from_secs(3600);

    fn timing(min_gap: Duration, keep_alive: Duration) -> Timing {
        Timing {
            min_gap,
            settle: Duration::ZERO,
            keep_alive,
        }
    }

    fn song(id: &str) -> Song {
        serde_json::from_value(json!({
            "id": id,
            "title": "Blue in Green",
            "artist": "Miles Davis",
            "album": "Kind of Blue",
            "coverArt": format!("cover-{id}"),
        }))
        .unwrap()
    }

    fn live_link(url: &str) -> CoverLink {
        CoverLink {
            url: url.to_owned(),
            expires: SystemTime::now() + Duration::from_secs(60 * 60),
        }
    }

    fn listening(title: &str) -> Listening {
        Listening {
            title: Some(title.to_owned()),
            line: None,
            started: 1_000,
            ends: None,
            cover: None,
        }
    }

    struct FakeDiscord {
        frames: Arc<Mutex<Vec<Value>>>,
        /// The opcode to answer each frame with; `None` is a broken socket.
        answers: VecDeque<Option<u32>>,
    }

    impl DiscordIpc for FakeDiscord {
        fn get_client_id(&self) -> &str {
            CLIENT_ID
        }
        fn connect_ipc(&mut self) -> Result<(), Error> {
            Ok(())
        }
        fn send(&mut self, data: Value, _opcode: u8) -> Result<(), Error> {
            self.frames.lock().unwrap().push(data);
            Ok(())
        }
        fn write(&mut self, _data: &[u8]) -> Result<(), Error> {
            unreachable!("`send` is replaced")
        }
        fn recv(&mut self) -> Result<(u32, Value), Error> {
            match self.answers.pop_front().flatten() {
                Some(opcode) => Ok((opcode, json!({}))),
                None => Err(Error::NotConnected),
            }
        }
        fn read(&mut self, _buffer: &mut [u8]) -> Result<(), Error> {
            unreachable!("`recv` is replaced")
        }
        fn close(&mut self) -> Result<(), Error> {
            Ok(())
        }
    }

    fn fake_connections(
        connections: Vec<Vec<Option<u32>>>,
    ) -> (impl FnMut() -> Option<FakeDiscord>, Arc<Mutex<Vec<Value>>>) {
        let frames = Arc::new(Mutex::new(Vec::new()));
        let shared = frames.clone();
        let mut connections = VecDeque::from(connections);
        let connect = move || {
            connections.pop_front().map(|answers| FakeDiscord {
                frames: shared.clone(),
                answers: answers.into(),
            })
        };
        (connect, frames)
    }

    #[test]
    fn the_status_is_of_type_listening_with_the_song_and_its_progress() {
        let started = UNIX_EPOCH + Duration::from_secs(1_000);
        let listening = Listening::new(&song("1"), started, Some(Duration::from_secs(90)));
        let (mut connect, frames) = fake_connections(vec![vec![Some(1)]]);

        deliver(&mut None, Some(&listening), &mut connect);

        let frames = frames.lock().unwrap();
        assert_eq!(frames[0]["cmd"], "SET_ACTIVITY");
        assert_eq!(
            frames[0]["args"]["activity"],
            json!({
                "type": 2,
                "details": "Blue in Green",
                "state": "Miles Davis — Kind of Blue",
                "timestamps": { "start": 1_000_000, "end": 1_090_000 },
            })
        );
    }

    #[test]
    fn text_is_made_a_length_discord_accepts() {
        assert_eq!(fit(""), None);
        assert_eq!(fit("X").as_deref(), Some("X\u{200b}"));
        assert_eq!(fit("XY").as_deref(), Some("XY"));
        for long in ["a".repeat(129), "あ".repeat(500), "😀".repeat(128)] {
            let fitted = fit(&long).unwrap();
            assert!(
                TEXT_UNITS.contains(&fitted.encode_utf16().count()),
                "{fitted}"
            );
            assert!(fitted.ends_with('…'));
        }
        let exact = "😀".repeat(64);
        assert_eq!(fit(&exact).as_deref(), Some(exact.as_str()));
    }

    #[test]
    fn a_failed_connection_is_replaced_at_once() {
        let (mut connect, frames) = fake_connections(vec![vec![None], vec![Some(1)]]);
        let mut connection = None;

        deliver(&mut connection, Some(&listening("Song")), &mut connect);

        assert_eq!(frames.lock().unwrap().len(), 2);
        assert!(connection.is_some());
    }

    #[test]
    fn a_connection_discord_closed_is_replaced() {
        let (mut connect, frames) = fake_connections(vec![vec![Some(OPCODE_CLOSE)], vec![Some(1)]]);
        let mut connection = None;

        deliver(&mut connection, Some(&listening("Song")), &mut connect);

        assert_eq!(frames.lock().unwrap().len(), 2);
        assert!(connection.is_some());
    }

    #[test]
    fn without_discord_nothing_is_sent() {
        let (mut connect, frames) = fake_connections(vec![]);
        let mut connection = None;

        deliver(&mut connection, Some(&listening("Song")), &mut connect);

        assert!(frames.lock().unwrap().is_empty());
        assert!(connection.is_none());
    }

    #[test]
    fn clearing_does_not_connect() {
        let (mut connect, frames) = fake_connections(vec![vec![Some(1)]]);
        let mut connection = None;

        deliver(&mut connection, None, &mut connect);

        assert!(frames.lock().unwrap().is_empty());
        assert!(connection.is_none());
    }

    #[test]
    fn clearing_tells_an_open_connection() {
        let (mut connect, frames) = fake_connections(vec![vec![Some(1), Some(1)]]);
        let mut connection = None;

        deliver(&mut connection, Some(&listening("Song")), &mut connect);
        deliver(&mut connection, None, &mut connect);

        let frames = frames.lock().unwrap();
        assert_eq!(frames[1]["args"]["activity"], Value::Null);
        assert!(connection.is_some());
    }

    #[test]
    fn only_the_last_of_several_quick_updates_is_sent() {
        let (connect, frames) = fake_connections(vec![vec![Some(1)]]);
        let (updates, received) = mpsc::channel();
        updates.send(Some(listening("First"))).unwrap();
        updates.send(Some(listening("Last"))).unwrap();
        // Both are queued before the thread starts, so it takes in both first
        let run = thread::spawn(move || run(received, connect, timing(SHORT, NEVER)));
        while frames.lock().unwrap().is_empty() {
            thread::sleep(SHORT);
        }
        drop(updates);
        run.join().unwrap();

        let frames = frames.lock().unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0]["args"]["activity"]["details"], "Last");
    }

    #[test]
    fn the_status_is_sent_again_after_a_while() {
        let answers = vec![Some(1); 8];
        let (connect, frames) = fake_connections(vec![answers]);
        let (updates, received) = mpsc::channel();
        updates.send(Some(listening("Song"))).unwrap();
        let run = thread::spawn(move || run(received, connect, timing(SHORT, SHORT)));
        while frames.lock().unwrap().len() < 2 {
            thread::sleep(SHORT);
        }
        drop(updates);
        run.join().unwrap();
    }

    #[test]
    fn a_keep_alive_does_not_hold_back_the_first_status() {
        let (connect, frames) = fake_connections(vec![vec![Some(1); 64]]);
        let (updates, received) = mpsc::channel();
        let run = thread::spawn(move || run(received, connect, timing(NEVER, SHORT)));
        // Keep-alives pass with nothing to show
        thread::sleep(SHORT * 4);
        updates.send(Some(listening("Song"))).unwrap();
        let asked = Instant::now();
        while frames.lock().unwrap().is_empty() {
            assert!(asked.elapsed() < Duration::from_secs(5), "held back");
            thread::sleep(SHORT);
        }
        drop(updates);
        run.join().unwrap();
    }

    fn presence() -> (Presence, Receiver<Option<Listening>>) {
        let (updates, received) = mpsc::channel();
        (Presence::sending_to(updates), received)
    }

    #[test]
    fn the_same_song_playing_on_is_not_sent_again() {
        let (mut presence, received) = presence();
        let song = song("1");

        presence.listening(&song, Duration::from_secs(10), None);
        presence.listening(&song, Duration::from_secs(10), None);

        assert_eq!(received.try_iter().count(), 1);
    }

    #[test]
    fn a_seek_or_a_new_song_is_sent() {
        let (mut presence, received) = presence();

        presence.listening(&song("1"), Duration::from_secs(10), None);
        presence.listening(&song("1"), Duration::from_secs(60), None);
        presence.listening(&song("2"), Duration::from_secs(60), None);

        assert_eq!(received.try_iter().count(), 3);
    }

    #[test]
    fn clearing_is_sent_once_and_playing_on_shows_it_again() {
        let (mut presence, received) = presence();
        let song = song("1");

        presence.clear();
        assert_eq!(received.try_iter().count(), 0);

        presence.listening(&song, Duration::ZERO, None);
        presence.clear();
        presence.clear();
        presence.listening(&song, Duration::ZERO, None);

        let sent: Vec<_> = received.try_iter().map(|update| update.is_some()).collect();
        assert_eq!(sent, [true, false, true]);
    }

    #[test]
    fn turned_off_the_status_goes_away_and_stays_away_until_turned_on() {
        let (mut presence, received) = presence();
        let song = song("1");
        presence.listening(&song, Duration::ZERO, None);

        presence.set_on(false);
        presence.listening(&song, Duration::ZERO, None);
        presence.set_cover(CoverArtId::new("cover-1"), live_link("https://i.ibb.co/1"));

        let sent: Vec<_> = received.try_iter().map(|update| update.is_some()).collect();
        assert_eq!(sent, [true, false]);

        presence.set_on(true);
        assert_eq!(received.try_iter().count(), 0);
        presence.listening(&song, Duration::ZERO, None);
        let sent: Vec<_> = received.try_iter().collect();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].is_some());
    }

    #[test]
    fn a_cover_goes_on_the_status_as_a_large_image() {
        let mut listening = listening("Song");
        listening.cover = Some("https://i.ibb.co/abc.png".to_owned());
        let (mut connect, frames) = fake_connections(vec![vec![Some(1)]]);

        deliver(&mut None, Some(&listening), &mut connect);

        assert_eq!(
            frames.lock().unwrap()[0]["args"]["activity"]["assets"],
            json!({ "large_image": "https://i.ibb.co/abc.png" })
        );
    }

    #[test]
    fn a_cover_found_later_is_sent_for_the_song_showing() {
        let (mut presence, received) = presence();
        let song = song("1");
        presence.listening(&song, Duration::ZERO, None);

        presence.set_cover(CoverArtId::new("cover-1"), live_link("https://i.ibb.co/1"));

        let sent: Vec<_> = received.try_iter().flatten().collect();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0].cover, None);
        assert_eq!(sent[1].cover.as_deref(), Some("https://i.ibb.co/1"));
    }

    #[test]
    fn a_cover_found_for_another_song_is_kept_for_when_it_plays() {
        let (mut presence, received) = presence();
        presence.listening(&song("1"), Duration::ZERO, None);

        presence.set_cover(CoverArtId::new("cover-2"), live_link("https://i.ibb.co/2"));
        assert_eq!(received.try_iter().count(), 1);
        assert!(presence.has_cover(&CoverArtId::new("cover-2")));

        presence.listening(&song("2"), Duration::ZERO, None);
        let sent: Vec<_> = received.try_iter().flatten().collect();
        assert_eq!(sent[0].cover.as_deref(), Some("https://i.ibb.co/2"));
    }

    #[test]
    fn a_cover_stays_through_a_seek_and_a_pause() {
        let (mut presence, received) = presence();
        let song = song("1");
        presence.set_cover(CoverArtId::new("cover-1"), live_link("https://i.ibb.co/1"));

        presence.listening(&song, Duration::ZERO, None);
        presence.listening(&song, Duration::from_secs(60), None);
        presence.clear();
        presence.listening(&song, Duration::from_secs(60), None);

        let covers: Vec<_> = received
            .try_iter()
            .flatten()
            .map(|listening| listening.cover)
            .collect();
        assert_eq!(covers.len(), 3);
        assert!(covers.iter().all(|cover| cover.is_some()));
    }

    #[test]
    fn a_link_past_its_time_is_not_used() {
        let (mut presence, received) = presence();
        let cover = CoverArtId::new("cover-1");
        let mut link = live_link("https://i.ibb.co/1");
        link.expires = SystemTime::now() - Duration::from_secs(1);
        presence.set_cover(cover.clone(), link);
        assert!(!presence.has_cover(&cover));

        presence.listening(&song("1"), Duration::ZERO, None);

        let sent: Vec<_> = received.try_iter().flatten().collect();
        assert_eq!(sent[0].cover, None);
    }

    #[test]
    fn a_change_waits_to_settle_so_that_a_cover_goes_with_its_song() {
        let (connect, frames) = fake_connections(vec![vec![Some(1); 8]]);
        let (updates, received) = mpsc::channel();
        let timing = Timing {
            settle: Duration::from_millis(200),
            ..timing(Duration::ZERO, NEVER)
        };
        let run = thread::spawn(move || run(received, connect, timing));
        let mut with_cover = listening("Song");
        with_cover.cover = Some("https://i.ibb.co/1".into());

        updates.send(Some(listening("Song"))).unwrap();
        updates.send(Some(with_cover)).unwrap();
        while frames.lock().unwrap().is_empty() {
            thread::sleep(SHORT);
        }
        drop(updates);
        run.join().unwrap();

        let frames = frames.lock().unwrap();
        assert_eq!(frames.len(), 1);
        assert!(frames[0]["args"]["activity"]["assets"].is_object());
    }
}
