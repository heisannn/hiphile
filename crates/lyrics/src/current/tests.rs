use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use executor::testing::TestExecutor;
use serde_json::json;

use super::*;

fn song(id: &str) -> Song {
    serde_json::from_value(json!({
        "id": id,
        "title": format!("Title of {id}"),
        "artist": "Artist",
        "album": "Album",
    }))
    .unwrap()
}

fn lyrics(lines: usize) -> Lyrics {
    let body: Vec<_> = (0..lines)
        .map(|ix| {
            let start = 10. + ix as f64 * 5.;
            json!({
                "ts": start,
                "te": start + 4.5,
                "x": format!("line {ix}"),
                "l": [
                    { "c": "line ", "o": 0.0 },
                    { "c": format!("{ix}"), "o": 2.0 },
                ],
            })
        })
        .collect();
    crate::lyrics_from_richsync_body(&serde_json::Value::Array(body).to_string())
}

/// What lookups were asked about, and what they find; once those run out, a lookup finds
/// nothing for as long as the test goes on.
#[derive(Clone, Default)]
struct Finds {
    asked: Arc<Mutex<Vec<String>>>,
    answers: Arc<Mutex<VecDeque<Result<Lyrics, Error>>>>,
}

impl Finds {
    fn answer_with(&self, answer: Result<Lyrics, Error>) {
        self.answers.lock().unwrap().push_back(answer);
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }

    fn source(&self) -> FindLyrics {
        let finds = self.clone();
        Arc::new(move |query| {
            finds.asked.lock().unwrap().push(format!("{query:?}"));
            match finds.answers.lock().unwrap().pop_front() {
                Some(answer) => futures::future::ready(answer).boxed(),
                None => futures::future::pending().boxed(),
            }
        })
    }
}

fn current(cx: &TestExecutor) -> (CurrentLyrics, Finds) {
    let finds = Finds::default();
    let executor = cx.executor();
    (
        CurrentLyrics::with_find_lyrics(executor, finds.source()),
        finds,
    )
}

fn lookup(current: &CurrentLyrics) -> Lookup {
    current.snapshot().lookup().clone()
}

#[test]
fn a_new_song_starts_over_and_found_lyrics_are_kept() {
    let cx = &TestExecutor::new();
    let (current, finds) = current(cx);
    let _wanted = current.want();
    finds.answer_with(Ok(lyrics(3)));
    current.set_song(Some(&song("a")));
    cx.run_until_parked();
    assert!(matches!(lookup(&current), Lookup::Done(song, _) if song.to_string() == "a"));

    current.set_song(Some(&song("b")));
    assert!(matches!(lookup(&current), Lookup::Loading(song) if song.to_string() == "b"));
    assert_eq!(finds.asked().len(), 2);

    // An answer about the song before is out of date
    current.finish_lookup(SongId::new("a"), Ok(Lyrics::NotFound));
    assert!(matches!(lookup(&current), Lookup::Loading(_)));
    assert_eq!(current.0.state.read(|state| state.found.len()), 1);

    // Back to the first song, whose lyrics are at hand
    current.set_song(Some(&song("a")));
    assert!(matches!(lookup(&current), Lookup::Done(song, _) if song.to_string() == "a"));
    assert_eq!(finds.asked().len(), 2);
}

#[test]
fn an_answer_after_the_song_changed_is_kept_for_going_back() {
    let cx = &TestExecutor::new();
    let (current, finds) = current(cx);
    let _wanted = current.want();
    current.set_song(Some(&song("a")));
    current.set_song(Some(&song("b")));

    current.finish_lookup(SongId::new("a"), Ok(lyrics(2)));
    assert!(matches!(lookup(&current), Lookup::Loading(s) if s.to_string() == "b"));

    current.set_song(Some(&song("a")));
    assert!(matches!(lookup(&current), Lookup::Done(..)));
    assert_eq!(finds.asked().len(), 2);
}

#[test]
fn wanting_looks_for_the_current_song_once() {
    let cx = &TestExecutor::new();
    let (current, finds) = current(cx);
    current.set_song(Some(&song("a")));
    assert!(finds.asked().is_empty(), "unwanted, nothing is looked for");

    drop(current.want());
    let _wanted = current.want();

    assert_eq!(finds.asked().len(), 1);
    assert!(finds.asked()[0].contains("Title of a"));
}

#[test]
fn all_that_want_the_lyrics_share_one_lookup() {
    let cx = &TestExecutor::new();
    let (current, finds) = current(cx);
    current.set_song(Some(&song("a")));
    let first = current.want();
    let _second = current.want();
    assert_eq!(finds.asked().len(), 1);

    drop(first);
    current.set_song(Some(&song("b")));
    assert_eq!(finds.asked().len(), 2);
}

#[test]
fn a_song_that_comes_while_no_one_wants_the_lyrics_waits_until_someone_does() {
    let cx = &TestExecutor::new();
    let (current, finds) = current(cx);
    current.set_song(Some(&song("a")));
    drop(current.want());
    current.set_song(Some(&song("b")));
    assert_eq!(finds.asked().len(), 1);

    let _wanted = current.want();
    assert_eq!(finds.asked().len(), 2);
    assert!(finds.asked()[1].contains("Title of b"));

    current.set_song(None);
    assert_eq!(lookup(&current), Lookup::Nothing);
}

#[test]
fn a_failed_lookup_is_tried_again_only_when_asked() {
    let cx = &TestExecutor::new();
    let (current, finds) = current(cx);
    finds.answer_with(Err(Error::InvalidResponse));
    current.set_song(Some(&song("a")));
    let wanted = current.want();
    cx.run_until_parked();
    assert!(matches!(lookup(&current), Lookup::Failed(..)));

    // As when the song pauses or plays on, or the lyrics close and open again
    current.set_song(Some(&song("a")));
    drop(wanted);
    let _wanted = current.want();
    assert!(matches!(lookup(&current), Lookup::Failed(..)));
    assert_eq!(finds.asked().len(), 1);

    finds.answer_with(Ok(lyrics(2)));
    current.retry();
    cx.run_until_parked();
    assert!(matches!(lookup(&current), Lookup::Done(..)));
    assert_eq!(finds.asked().len(), 2);

    current.retry();
    assert_eq!(finds.asked().len(), 2);
}

#[test]
fn a_song_asks_for_what_it_knows() {
    let asked = query_for(&song("a"));
    assert_eq!(
        asked,
        TrackQuery::new("Title of a")
            .artist(Some("Artist"))
            .album(Some("Album"))
    );

    let song: Song = serde_json::from_value(json!({
        "id": "b",
        "title": "A Title",
        "artist": "An Artist",
        "displayAlbumArtist": "An Album Artist",
        "duration": 200,
    }))
    .unwrap();
    assert_eq!(
        query_for(&song),
        TrackQuery::new("A Title")
            .artist(Some("An Artist"))
            .album_artist(Some("An Album Artist"))
            .duration(Some(Duration::from_secs(200)))
    );
}

#[test]
fn a_length_of_zero_is_no_length() {
    let song: Song =
        serde_json::from_value(json!({ "id": "a", "title": "A Title", "duration": 0 })).unwrap();
    assert_eq!(query_for(&song), TrackQuery::new("A Title"));
}

#[test]
fn errors_say_what_to_try() {
    let net = net::Error::Canceled;
    assert!(describe(&Error::Network(net)).contains("connection"));
    assert!(describe(&Error::InvalidResponse).contains("later"));
}
