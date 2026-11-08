use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use gpui_kit::TestAppContext;
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

fn current(cx: &mut TestAppContext) -> (CurrentLyrics, Finds) {
    let finds = Finds::default();
    let executor = cx.update(|cx| gpui_host::executor(cx));
    (
        CurrentLyrics::with_find_lyrics(executor, finds.source()),
        finds,
    )
}

fn lookup(current: &CurrentLyrics) -> Lookup {
    current.snapshot().lookup().clone()
}

#[gpui_kit::test]
fn a_new_song_starts_over_and_found_lyrics_are_kept(cx: &mut TestAppContext) {
    let (current, finds) = current(cx);
    finds.answer_with(Ok(lyrics(3)));
    current.look_up(Some(&song("a")));
    cx.run_until_parked();
    assert!(matches!(lookup(&current), Lookup::Done(song, _) if song.to_string() == "a"));

    current.look_up(Some(&song("b")));
    assert!(matches!(lookup(&current), Lookup::Loading(song) if song.to_string() == "b"));
    assert_eq!(finds.asked().len(), 2);

    // An answer about the song before is out of date
    current.finish_lookup(SongId::new("a"), Ok(Lyrics::NotFound));
    assert!(matches!(lookup(&current), Lookup::Loading(_)));
    assert_eq!(current.0.state.read(|state| state.found.len()), 1);

    // Back to the first song, whose lyrics are at hand
    current.look_up(Some(&song("a")));
    assert!(matches!(lookup(&current), Lookup::Done(song, _) if song.to_string() == "a"));
    assert_eq!(finds.asked().len(), 2);
}

#[gpui_kit::test]
fn an_answer_after_the_song_changed_is_kept_for_going_back(cx: &mut TestAppContext) {
    let (current, finds) = current(cx);
    current.look_up(Some(&song("a")));
    current.look_up(Some(&song("b")));

    current.finish_lookup(SongId::new("a"), Ok(lyrics(2)));
    assert!(matches!(lookup(&current), Lookup::Loading(s) if s.to_string() == "b"));

    current.look_up(Some(&song("a")));
    assert!(matches!(lookup(&current), Lookup::Done(..)));
    assert_eq!(finds.asked().len(), 2);
}

#[gpui_kit::test]
fn a_failed_lookup_is_tried_again(cx: &mut TestAppContext) {
    let (current, finds) = current(cx);
    finds.answer_with(Err(Error::InvalidResponse));
    current.look_up(Some(&song("a")));
    cx.run_until_parked();
    assert!(matches!(lookup(&current), Lookup::Failed(..)));

    finds.answer_with(Ok(lyrics(2)));
    current.look_up(Some(&song("a")));
    cx.run_until_parked();
    assert!(matches!(lookup(&current), Lookup::Done(..)));
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
