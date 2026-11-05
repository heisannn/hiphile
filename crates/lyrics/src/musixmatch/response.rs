//! Reads what the service answers. It leaves fields out and changes their
//! types from one answer to another, so bodies stay as loose JSON until their
//! status is known to be a success.

use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use crate::matching::Offered;
use crate::model::{Lyrics, PlainLyrics, RawLine, TimedLyrics, timed_lines};
use crate::{Error, TrackQuery};

#[derive(Debug, Deserialize)]
struct Envelope {
    message: Message,
}

#[derive(Debug, Deserialize)]
struct Message {
    header: Header,
    #[serde(default)]
    body: Value,
}

#[derive(Debug, Deserialize)]
struct Header {
    status_code: u16,
    #[serde(default)]
    hint: Value,
}

impl Message {
    fn into_body(self) -> Result<Value, Error> {
        if self.header.status_code == 200 {
            Ok(self.body)
        } else {
            Err(Error::Api {
                status: self.header.status_code.to_string(),
                hint: self.header.hint.as_str().map(str::to_owned),
            })
        }
    }
}

pub(crate) fn token(body: &[u8]) -> Result<String, Error> {
    let envelope: Envelope = serde_json::from_slice(body).map_err(|_| Error::InvalidResponse)?;
    let body = envelope.message.into_body()?;
    body.get("user_token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .ok_or(Error::InvalidResponse)
}

#[derive(Debug, PartialEq)]
enum Part<T> {
    Found(T),
    Absent,
    Restricted,
    /// The service failed at this, which says nothing about the lyrics.
    Failed,
}

pub(crate) fn lyrics(body: &[u8], query: &TrackQuery) -> Result<Lyrics, Error> {
    let envelope: Envelope = serde_json::from_slice(body).map_err(|_| Error::InvalidResponse)?;
    let status = envelope.message.header.status_code;
    if status == 404 {
        return Ok(Lyrics::NotFound);
    }
    let body = envelope.message.into_body()?;
    let calls = body.get("macro_calls");
    let call = |name: &str| {
        let message = calls?.get(name)?.get("message")?;
        Message::deserialize(message).ok()
    };

    let track = call("matcher.track.get");
    if track
        .as_ref()
        .is_some_and(|track| track.header.status_code == 404)
    {
        return Ok(Lyrics::NotFound);
    }
    let track = track.and_then(|track| track.into_body().ok());
    let track = track.as_ref().and_then(|body| body.get("track"));
    let offered = track.map(offered);
    if offered
        .as_ref()
        .is_some_and(|offered| !offered.may_be_the_song(query))
    {
        return Ok(Lyrics::NotFound);
    }
    let instrumental = track
        .and_then(|track| track.get("instrumental"))
        .is_some_and(is_set);
    let times_can_be_trusted = !offered
        .as_ref()
        .is_some_and(|offered| offered.is_another_length(query));

    let parts = [
        call("track.richsync.get").map_or(Part::Absent, read_richsync),
        call("track.subtitles.get").map_or(Part::Absent, read_subtitles),
    ];
    let plain = call("track.lyrics.get").map_or(Part::Absent, read_plain);
    let mut restricted = plain == Part::Restricted;
    let mut failed = plain == Part::Failed;
    for part in parts {
        match part {
            Part::Found(lyrics) if times_can_be_trusted => return Ok(Lyrics::Timed(lyrics)),
            Part::Restricted => restricted = true,
            Part::Failed => failed = true,
            Part::Found(_) | Part::Absent => {}
        }
    }
    if let Part::Found(plain) = plain {
        return Ok(Lyrics::Plain(plain));
    }
    if instrumental {
        Ok(Lyrics::Instrumental)
    } else if restricted {
        Ok(Lyrics::Restricted)
    } else if failed {
        Err(Error::InvalidResponse)
    } else {
        Ok(Lyrics::NotFound)
    }
}

fn offered(track: &Value) -> Offered<'_> {
    let name = |key| {
        track
            .get(key)
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
    };
    Offered {
        title: name("track_name"),
        artist: name("artist_name"),
        length: track
            .get("track_length")
            .and_then(Value::as_f64)
            .filter(|length| *length > 0.)
            .and_then(|length| Duration::try_from_secs_f64(length).ok()),
    }
}

/// A flag, which comes as a number or a boolean.
fn is_set(value: &Value) -> bool {
    match value {
        Value::Bool(set) => *set,
        Value::Number(number) => number.as_f64().is_some_and(|number| number != 0.),
        _ => false,
    }
}

fn is_restricted(value: &Value) -> bool {
    value.get("restricted").is_some_and(is_set)
}

fn copyright(value: &Value) -> Option<Arc<str>> {
    value
        .get("lyrics_copyright")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(Arc::from)
}

fn part_body<T>(message: Message) -> Result<Value, Part<T>> {
    match message.header.status_code {
        200 => Ok(message.body),
        404 => Err(Part::Absent),
        _ => Err(Part::Failed),
    }
}

fn read_plain(message: Message) -> Part<PlainLyrics> {
    let body = match part_body(message) {
        Ok(body) => body,
        Err(part) => return part,
    };
    let Some(lyrics) = body.get("lyrics") else {
        return Part::Absent;
    };
    if is_restricted(lyrics) {
        return Part::Restricted;
    }
    let text = lyrics.get("lyrics_body").and_then(Value::as_str);
    match text
        .map(str::trim_end)
        .filter(|text| !text.trim().is_empty())
    {
        Some(text) => Part::Found(PlainLyrics::new(text, copyright(lyrics))),
        None => Part::Absent,
    }
}

#[derive(Deserialize)]
struct Subtitle {
    text: Option<String>,
    time: Option<SubtitleTime>,
}

#[derive(Deserialize)]
struct SubtitleTime {
    total: Option<f64>,
}

fn read_subtitles(message: Message) -> Part<TimedLyrics> {
    let body = match part_body(message) {
        Ok(body) => body,
        Err(part) => return part,
    };
    let list = body.get("subtitle_list").and_then(Value::as_array);
    let subtitles = list
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("subtitle"));
    let mut restricted = false;
    for subtitle in subtitles {
        if is_restricted(subtitle) {
            restricted = true;
            continue;
        }
        let Some(raw) = subtitle
            .get("subtitle_body")
            .and_then(Value::as_str)
            .and_then(|body| serde_json::from_str::<Vec<Subtitle>>(body).ok())
        else {
            return Part::Failed;
        };
        let lines = timed_lines(
            raw.into_iter()
                .map(|line| RawLine {
                    start: line.time.and_then(|time| time.total),
                    text: line.text,
                    ..RawLine::default()
                })
                .collect(),
        );
        if !lines.is_empty() {
            return Part::Found(TimedLyrics::new(lines, copyright(subtitle)));
        }
    }
    if restricted {
        Part::Restricted
    } else {
        Part::Absent
    }
}

#[derive(Deserialize)]
struct RichLine {
    ts: Option<f64>,
    te: Option<f64>,
    x: Option<String>,
    #[serde(default)]
    l: Vec<RichElement>,
}

#[derive(Deserialize)]
struct RichElement {
    c: Option<String>,
    o: Option<f64>,
}

fn read_richsync(message: Message) -> Part<TimedLyrics> {
    let body = match part_body(message) {
        Ok(body) => body,
        Err(part) => return part,
    };
    let Some(richsync) = body.get("richsync") else {
        return Part::Absent;
    };
    if is_restricted(richsync) {
        return Part::Restricted;
    }
    let Some(text) = richsync.get("richsync_body").and_then(Value::as_str) else {
        return Part::Absent;
    };
    let Ok(raw) = serde_json::from_str::<Vec<RichLine>>(text) else {
        return Part::Failed;
    };
    let lines = timed_lines(
        raw.into_iter()
            .map(|line| RawLine {
                start: line.ts,
                end: line.te,
                text: line.x,
                elements: line.l.into_iter().map(|el| (el.c, el.o)).collect(),
            })
            .collect(),
    );
    if lines.is_empty() {
        Part::Absent
    } else {
        Part::Found(TimedLyrics::new(lines, copyright(richsync)))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn message(status: u16, body: Value) -> Value {
        json!({ "message": { "header": { "status_code": status }, "body": body } })
    }

    fn matched() -> (&'static str, Value) {
        (
            "matcher.track.get",
            message(200, json!({ "track": { "track_length": 100 } })),
        )
    }

    fn richsync() -> (&'static str, Value) {
        let body = json!([{
            "ts": 1.0, "te": 3.0, "x": "word sync",
            "l": [{ "c": "word ", "o": 0.0 }, { "c": "sync", "o": 1.0 }],
        }]);
        (
            "track.richsync.get",
            message(
                200,
                json!({ "richsync": {
                    "richsync_body": body.to_string(),
                    "lyrics_copyright": "  Rich copyright\n",
                    "richssync_language": "en",
                } }),
            ),
        )
    }

    fn subtitles() -> (&'static str, Value) {
        let body = json!([{ "text": "line sync", "time": { "total": 2.0, "seconds": 2 } }]);
        (
            "track.subtitles.get",
            message(
                200,
                json!({ "subtitle_list": [{ "subtitle": {
                    "subtitle_body": body.to_string(),
                    "restricted": 0,
                } }] }),
            ),
        )
    }

    fn plain() -> (&'static str, Value) {
        (
            "track.lyrics.get",
            message(
                200,
                json!({ "lyrics": { "lyrics_body": "plain\n", "lyrics_copyright": "Plain" } }),
            ),
        )
    }

    fn answer(calls: Vec<(&'static str, Value)>) -> Vec<u8> {
        let calls: serde_json::Map<String, Value> = calls
            .into_iter()
            .map(|(name, call)| (name.to_owned(), call))
            .collect();
        message(200, json!({ "macro_calls": calls }))
            .to_string()
            .into_bytes()
    }

    fn read(calls: Vec<(&'static str, Value)>) -> Lyrics {
        let query = TrackQuery::new("A Song").duration(Some(Duration::from_secs(100)));
        lyrics(&answer(calls), &query).unwrap()
    }

    fn answer_naming(name: &str, artist: &str, track_length: u64) -> Vec<u8> {
        let track = json!({
            "track": { "track_name": name, "artist_name": artist, "track_length": track_length },
        });
        answer(vec![("matcher.track.get", message(200, track)), plain()])
    }

    #[test]
    fn a_track_with_other_names_is_not_the_song() {
        let query = TrackQuery::new("Lemon")
            .artist(Some("米津玄師"))
            .duration(Some(Duration::from_secs(256)));

        let other_song = answer_naming("Orange", "Someone Else", 256);
        assert_eq!(lyrics(&other_song, &query).unwrap(), Lyrics::NotFound);

        let other_artist = answer_naming("Lemon", "Someone Else", 200);
        assert_eq!(lyrics(&other_artist, &query).unwrap(), Lyrics::NotFound);

        let other_script = answer_naming("Lemon", "Kenshi Yonezu", 256);
        assert!(matches!(
            lyrics(&other_script, &query),
            Ok(Lyrics::Plain(_))
        ));
    }

    #[test]
    fn an_instrumental_track_with_other_names_says_nothing_of_the_song() {
        let query = TrackQuery::new("Lemon").artist(Some("米津玄師"));
        let track = json!({ "track": {
            "track_name": "Orange",
            "artist_name": "Someone Else",
            "instrumental": 1,
        } });
        let bytes = answer(vec![("matcher.track.get", message(200, track))]);
        assert_eq!(lyrics(&bytes, &query).unwrap(), Lyrics::NotFound);
    }

    fn timed_text(lyrics: &Lyrics) -> &str {
        match lyrics {
            Lyrics::Timed(lyrics) => lyrics.lines()[0].text(),
            other => panic!("expected timed lyrics, got {other:?}"),
        }
    }

    #[test]
    fn the_finest_timing_found_wins() {
        let found = read(vec![matched(), plain(), subtitles(), richsync()]);
        assert_eq!(timed_text(&found), "word sync");
        let Lyrics::Timed(lyrics) = &found else {
            unreachable!()
        };
        assert_eq!(lyrics.lines()[0].elements().len(), 2);
        assert_eq!(
            lyrics.copyright().map(|text| &**text),
            Some("Rich copyright")
        );

        let found = read(vec![matched(), plain(), subtitles()]);
        assert_eq!(timed_text(&found), "line sync");

        let found = read(vec![matched(), plain()]);
        let Lyrics::Plain(plain) = found else {
            panic!("expected plain lyrics, got {found:?}")
        };
        assert_eq!(&**plain.text(), "plain");
        assert_eq!(plain.copyright().map(|text| &**text), Some("Plain"));
    }

    #[test]
    fn one_part_failing_does_not_lose_the_others() {
        let found = read(vec![
            matched(),
            ("track.richsync.get", message(500, json!(null))),
            subtitles(),
        ]);
        assert_eq!(timed_text(&found), "line sync");

        let found = read(vec![
            matched(),
            (
                "track.richsync.get",
                message(200, json!({ "richsync": { "richsync_body": "not json" } })),
            ),
            subtitles(),
        ]);
        assert_eq!(timed_text(&found), "line sync");
    }

    #[test]
    fn a_track_of_another_length_keeps_its_words_but_not_its_times() {
        let track = (
            "matcher.track.get",
            message(200, json!({ "track": { "track_length": 140 } })),
        );
        let found = read(vec![track.clone(), richsync(), plain()]);
        assert!(matches!(found, Lyrics::Plain(_)), "{found:?}");

        let found = read(vec![track, richsync()]);
        assert_eq!(found, Lyrics::NotFound);

        let without_duration = lyrics(&answer(vec![richsync()]), &TrackQuery::default()).unwrap();
        assert_eq!(timed_text(&without_duration), "word sync");
    }

    #[test]
    fn restricted_lyrics_are_not_shown() {
        let restricted = (
            "track.lyrics.get",
            message(
                200,
                json!({ "lyrics": { "lyrics_body": "hidden", "restricted": 1 } }),
            ),
        );
        assert_eq!(read(vec![matched(), restricted]), Lyrics::Restricted);

        let restricted = (
            "track.richsync.get",
            message(
                200,
                json!({ "richsync": { "richsync_body": "[]", "restricted": true } }),
            ),
        );
        assert_eq!(read(vec![matched(), restricted]), Lyrics::Restricted);
    }

    #[test]
    fn an_instrumental_track_says_so() {
        let track = (
            "matcher.track.get",
            message(200, json!({ "track": { "instrumental": 1 } })),
        );
        assert_eq!(read(vec![track]), Lyrics::Instrumental);
    }

    #[test]
    fn nothing_found_comes_in_many_shapes() {
        assert_eq!(
            read(vec![
                ("matcher.track.get", message(404, json!(null))),
                plain()
            ]),
            Lyrics::NotFound
        );
        assert_eq!(read(vec![matched()]), Lyrics::NotFound);
        assert_eq!(read(vec![]), Lyrics::NotFound);
        assert_eq!(
            read(vec![
                matched(),
                ("track.lyrics.get", message(404, json!([]))),
                ("track.subtitles.get", message(404, json!({}))),
                ("not a call", json!("unexpected")),
            ]),
            Lyrics::NotFound
        );
        let outer = message(404, json!([])).to_string().into_bytes();
        assert_eq!(
            lyrics(&outer, &TrackQuery::default()).unwrap(),
            Lyrics::NotFound
        );
        let empty = message(200, json!(null)).to_string().into_bytes();
        assert_eq!(
            lyrics(&empty, &TrackQuery::default()).unwrap(),
            Lyrics::NotFound
        );
    }

    #[test]
    fn every_part_failing_is_an_error() {
        let bytes = answer(vec![
            matched(),
            ("track.lyrics.get", message(500, json!(null))),
        ]);
        assert!(matches!(
            lyrics(&bytes, &TrackQuery::default()),
            Err(Error::InvalidResponse)
        ));
    }

    #[test]
    fn a_refusal_carries_its_hint() {
        let bytes = json!({ "message": {
            "header": { "status_code": 401, "hint": "renew" }, "body": []
        } })
        .to_string()
        .into_bytes();
        let Err(Error::Api { status, hint }) = lyrics(&bytes, &TrackQuery::default()) else {
            panic!("expected a refusal");
        };
        assert_eq!((status.as_str(), hint.as_deref()), ("401", Some("renew")));
        assert!(matches!(token(&bytes), Err(Error::Api { status, .. }) if status == "401"));
    }

    #[test]
    fn answers_that_are_not_what_they_should_be_are_errors() {
        assert!(matches!(
            lyrics(b"<html>", &TrackQuery::default()),
            Err(Error::InvalidResponse)
        ));
        let no_token = message(200, json!({})).to_string().into_bytes();
        assert!(matches!(token(&no_token), Err(Error::InvalidResponse)));
        let empty_token = message(200, json!({ "user_token": "" }))
            .to_string()
            .into_bytes();
        assert!(matches!(token(&empty_token), Err(Error::InvalidResponse)));
    }
}
