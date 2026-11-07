use std::io::{Cursor, Read, Seek, SeekFrom};
use std::sync::mpsc;
use std::time::Instant;

use super::*;
use crate::test_wav::{counting_frame, wav, wav_counting, wav_with};

const WAIT: Duration = Duration::from_secs(5);

fn wait_for<T>(mut poll: impl FnMut() -> Option<T>) -> T {
    let started = Instant::now();
    loop {
        if let Some(value) = poll() {
            return value;
        }
        assert!(started.elapsed() < WAIT, "still waiting");
        thread::sleep(Duration::from_millis(1));
    }
}

fn opened_feed(mut opening: Option<Opening>) -> Feed {
    wait_for(|| match opening.take().unwrap().poll() {
        Polled::Waiting(waiting) => {
            opening = Some(waiting);
            None
        }
        Polled::Opened(feed) => Some(feed),
        Polled::Failed(error) => panic!("{error}"),
    })
}

fn start(track: Track, ahead: Duration) -> Result<Opening, Error> {
    Opening::start(track, ahead, Arc::new(AtomicBool::new(false)))
}

fn opened(track: Track, ahead: Duration) -> Feed {
    opened_feed(Some(start(track.extension("wav"), ahead).unwrap()))
}

/// One second at 8 kHz, whose samples each hold their frame number.
fn second() -> Vec<u8> {
    wav_counting(8_000, 1, 8_000)
}

fn counted(feed: &Feed) -> Vec<f32> {
    let mut all = Vec::new();
    loop {
        match wait_for(|| feed.take()) {
            Item::Samples(samples) => all.extend(samples),
            Item::Seeked(_) | Item::Skipped(_) => {}
            Item::End => return all,
            Item::Failed(error) => panic!("{error}"),
        }
    }
}

/// Sends what it holds, then blocks a read until told to go on.
struct Gated {
    bytes: Cursor<Vec<u8>>,
    open_until: u64,
    go_on: Mutex<mpsc::Receiver<()>>,
}

impl Read for Gated {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.bytes.position() >= self.open_until {
            self.go_on.lock().unwrap().recv().ok();
            self.open_until = u64::MAX;
        }
        let room = usize::try_from(self.open_until - self.bytes.position()).unwrap_or(usize::MAX);
        let len = buf.len().min(room);
        self.bytes.read(&mut buf[..len])
    }
}

impl Seek for Gated {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        self.bytes.seek(to)
    }
}

#[test]
fn decodes_a_track_from_start_to_end() {
    let feed = opened(Track::new(Cursor::new(second())), Duration::from_secs(5));
    assert_eq!(feed.spec().rate, 8_000);
    let samples = counted(&feed);
    assert_eq!(samples.len(), 8_000);
    assert_eq!(counting_frame(samples[0]), 0);
    assert!(feed.take().is_none());
}

#[test]
fn decodes_only_as_far_ahead_as_asked() {
    let bytes = wav(8_000, 1, 80_000, 0);
    let feed = opened(Track::new(Cursor::new(bytes)), Duration::from_millis(500));
    assert!(wait_for(|| feed
        .has_ahead(Duration::from_millis(500))
        .then_some(true)));
    thread::sleep(Duration::from_millis(50));
    let ready = feed.link.0.lock().samples;
    // A packet may cross the line, but it does not go on from there
    assert!(ready < 8_000, "{ready}");

    // Asking for more decodes further
    assert!(!feed.has_ahead(Duration::from_secs(2)));
    wait_for(|| feed.has_ahead(Duration::from_secs(2)).then_some(()));
}

#[test]
fn a_track_shorter_than_asked_counts_as_ready() {
    let feed = opened(Track::new(Cursor::new(second())), Duration::from_secs(5));
    wait_for(|| feed.has_ahead(Duration::from_secs(30)).then_some(()));
}

#[test]
fn nothing_waits_for_a_read_that_is_blocked() {
    let bytes = second();
    let (go_on, gate) = mpsc::channel();
    // The header and a little audio, but less than is asked for
    let source = Gated {
        bytes: Cursor::new(bytes),
        open_until: 4_096,
        go_on: Mutex::new(gate),
    };
    let feed = opened(Track::new(source), Duration::from_secs(5));

    let started = Instant::now();
    assert!(!feed.has_ahead(Duration::from_secs(1)));
    while feed.take().is_some() {}
    feed.seek(Duration::from_millis(500));
    assert!(feed.take().is_none());
    assert!(started.elapsed() < Duration::from_secs(1));

    go_on.send(()).unwrap();
    assert!(matches!(
        wait_for(|| feed.take()),
        Item::Seeked(Some(to)) if to == Duration::from_millis(500)
    ));
}

#[test]
fn a_seek_replaces_what_had_not_been_taken() {
    let feed = opened(Track::new(Cursor::new(second())), Duration::from_secs(5));
    wait_for(|| feed.has_ahead(Duration::from_secs(1)).then_some(()));

    feed.seek(Duration::from_millis(750));
    assert!(matches!(
        wait_for(|| feed.take()),
        Item::Seeked(Some(to)) if to == Duration::from_millis(750)
    ));
    let samples = counted(&feed);
    assert_eq!(samples.len(), 2_000);
    assert_eq!(counting_frame(samples[0]), 6_000);
}

#[test]
fn a_seek_goes_on_after_the_end() {
    let feed = opened(Track::new(Cursor::new(second())), Duration::from_secs(5));
    counted(&feed);
    feed.seek(Duration::ZERO);
    assert_eq!(counted(&feed).len(), 8_000);
}

#[test]
fn a_seek_past_the_end_reports_it() {
    let feed = opened(Track::new(Cursor::new(second())), Duration::from_secs(5));
    feed.seek(Duration::from_secs(2));
    assert!(matches!(wait_for(|| feed.take()), Item::Seeked(None)));
    assert!(feed.has_ahead(Duration::from_secs(30)));
}

#[test]
fn a_seek_made_while_opening_comes_first() {
    let opening = start(
        Track::new(Cursor::new(second())).extension("wav"),
        Duration::from_secs(5),
    )
    .unwrap();
    opening.seek(Duration::from_millis(250));
    let feed = opened_feed(Some(opening));
    assert!(matches!(
        wait_for(|| feed.take()),
        Item::Seeked(Some(to)) if to == Duration::from_millis(250)
    ));
    assert_eq!(counting_frame(counted(&feed)[0]), 2_000);
}

struct PanicsAtTheEnd(Cursor<Vec<u8>>);

impl Read for PanicsAtTheEnd {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self.0.read(buf)? {
            0 => panic!("the decoder's source gave way"),
            len => Ok(len),
        }
    }
}

impl Seek for PanicsAtTheEnd {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        self.0.seek(to)
    }
}

#[test]
fn a_decoder_that_panics_once_open_fails_the_track() {
    // Shorter than its header says, or the decoder would stop at its end
    let mut bytes = second();
    bytes.truncate(bytes.len() / 2);
    let feed = opened(
        Track::new(PanicsAtTheEnd(Cursor::new(bytes))),
        Duration::from_secs(5),
    );
    loop {
        match wait_for(|| feed.take()) {
            Item::Failed(_) => break,
            Item::End => panic!("the source panics before the end"),
            Item::Samples(_) | Item::Seeked(_) | Item::Skipped(_) => {}
        }
    }
    assert!(feed.has_ahead(Duration::from_secs(30)));
}

#[test]
fn a_decoder_that_panics_while_opening_fails_to_open() {
    let source = PanicsAtTheEnd(Cursor::new(Vec::new()));
    let track = Track::new(source).extension("wav");
    let mut opening = Some(start(track, Duration::from_secs(1)).unwrap());
    wait_for(|| match opening.take().unwrap().poll() {
        Polled::Waiting(waiting) => {
            opening = Some(waiting);
            None
        }
        Polled::Opened(_) => panic!("opened"),
        Polled::Failed(_) => Some(()),
    });
}

#[test]
fn data_that_is_not_audio_fails_to_open() {
    let mut opening = Some(
        start(
            Track::new(Cursor::new(b"not audio".to_vec())),
            Duration::from_secs(1),
        )
        .unwrap(),
    );
    wait_for(|| match opening.take().unwrap().poll() {
        Polled::Waiting(waiting) => {
            opening = Some(waiting);
            None
        }
        Polled::Opened(_) => panic!("opened"),
        Polled::Failed(_) => Some(()),
    });
}

fn trimmed(wav: Vec<u8>, ahead: Duration) -> Feed {
    let track = Track::new(Cursor::new(wav)).extension("wav");
    let trim = Arc::new(AtomicBool::new(true));
    opened_feed(Some(Opening::start(track, ahead, trim).unwrap()))
}

/// Half a second of silence, half of sound, and half of silence again, at 8 kHz.
fn sound_between_silence() -> Vec<u8> {
    wav_with(8_000, 1, 12_000, |frame| {
        if (4_000..8_000).contains(&frame) {
            1_000
        } else {
            0
        }
    })
}

#[test]
fn trimmed_silence_counts_for_nothing_ahead_and_the_track_says_where_it_starts() {
    let feed = trimmed(sound_between_silence(), Duration::from_secs(10));
    wait_for(|| feed.has_ahead(Duration::from_secs(10)).then_some(()));
    assert_eq!(feed.ready(), Duration::from_millis(500));
    assert_eq!(feed.time_left(), Some(Duration::from_millis(500)));
    assert_eq!(feed.resumes_at(), Duration::from_millis(500));
    assert_eq!(counted(&feed).len(), 4_000);
}

#[test]
fn a_seek_back_starts_at_the_sound_again() {
    let feed = trimmed(sound_between_silence(), Duration::from_secs(10));
    wait_for(|| feed.has_ahead(Duration::from_secs(10)).then_some(()));
    feed.seek(Duration::ZERO);
    assert_eq!(feed.time_left(), None);
    wait_for(|| feed.has_ahead(Duration::from_secs(10)).then_some(()));
    assert_eq!(feed.resumes_at(), Duration::from_millis(500));
    assert!(matches!(
        wait_for(|| feed.take()),
        Item::Seeked(Some(at)) if at == Duration::from_millis(500)
    ));
}

#[test]
fn silence_before_the_sound_is_told_as_skipped() {
    let feed = trimmed(sound_between_silence(), Duration::from_secs(10));
    assert!(matches!(
        wait_for(|| feed.take()),
        Item::Skipped(at) if at == Duration::from_millis(500)
    ));
}

#[test]
fn the_time_left_is_known_only_once_the_end_has_been_decoded() {
    let feed = opened(
        Track::new(Cursor::new(second())),
        Duration::from_millis(100),
    );
    wait_for(|| feed.has_ahead(Duration::from_millis(100)).then_some(()));
    assert_eq!(feed.time_left(), None);
    assert!(feed.ready() >= Duration::from_millis(100));
    wait_for(|| feed.has_ahead(Duration::from_secs(2)).then_some(()));
    assert_eq!(feed.time_left(), Some(Duration::from_secs(1)));
}
