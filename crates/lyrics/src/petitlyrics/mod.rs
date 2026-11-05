//! Finds a song's lyrics on PetitLyrics, through the API its apps use.
//!
//! The API is not a published one. It takes no token, and it matches songs loosely,
//! so what it offers is held up against the song asked for.

mod lsy;
mod response;

use bytes::Bytes;
use futures::FutureExt as _;
use futures::future::BoxFuture;
use url::Url;
use url::form_urlencoded::Serializer;

use crate::matching::{Offered, without_version};
use crate::model::{PlainLyrics, RawLine, TimedLyrics, timed_lines};
use crate::{Error, Lyrics, Provider, TrackQuery};
use response::{Candidate, Line, Payload};

const URL: &str = "https://p1.petitlyrics.com/api/GetPetitLyricsData.php";
const TERMINAL_TYPE: &str = "10";
const CLIENT_APP_ID: &str = "p1232089";
const MOST_CANDIDATES: &str = "10";
/// Asks for lyrics with times, which come in whatever form each song has.
const KIND_TIMED: &str = "3";
const KIND_TEXT: &str = "1";

#[derive(Clone)]
pub struct Client {
    net: net::Client,
    url: Url,
}

impl Client {
    pub fn new(net: net::Client) -> Self {
        Self {
            net,
            url: Url::parse(URL).expect("the service's address is valid"),
        }
    }

    /// The service goes by the title as written, and may have only words under
    /// "A Song (feat. Someone)" and times under "A Song". So without times, it is
    /// asked again by the title without its version.
    pub async fn find_lyrics(&self, query: &TrackQuery) -> Result<Lyrics, Error> {
        let found = self.find_titled(&query.title, query).await?;
        let plain_title = without_version(&query.title);
        if matches!(found, Lyrics::Timed(_)) || plain_title == query.title {
            return Ok(found);
        }
        match self.find_titled(plain_title, query).await {
            Ok(timed @ Lyrics::Timed(_)) => Ok(timed),
            Ok(lyrics) if found == Lyrics::NotFound => Ok(lyrics),
            Err(error) if found == Lyrics::NotFound => Err(error),
            Ok(_) | Err(_) => Ok(found),
        }
    }

    async fn find_titled(&self, title: &str, query: &TrackQuery) -> Result<Lyrics, Error> {
        let mut candidates: Vec<(Rank, Candidate)> = self
            .search(title, query, KIND_TIMED)
            .await?
            .into_iter()
            .filter(|candidate| offered(candidate).is_the_song(query))
            .map(|candidate| (Rank::of(&candidate, query), candidate))
            .collect();
        candidates.sort_by_key(|(rank, _)| *rank);

        let mut texts = None;
        for (_, candidate) in candidates {
            if let Some(lyrics) = self.lyrics_of(candidate, title, query, &mut texts).await? {
                return Ok(lyrics);
            }
        }
        Ok(Lyrics::NotFound)
    }

    /// Its words may need another request, which one answer serves for all candidates:
    /// `texts` holds it once there is one.
    async fn lyrics_of(
        &self,
        candidate: Candidate,
        title: &str,
        query: &TrackQuery,
        texts: &mut Option<Vec<Candidate>>,
    ) -> Result<Option<Lyrics>, Error> {
        let can_trust_times = !offered(&candidate).is_another_length(query);
        Ok(match candidate.lyrics {
            Payload::Text(text) => plain(&text),
            Payload::Lines(lines) if can_trust_times => timed(lines),
            Payload::Lines(lines) => {
                let lines: Vec<String> = lines.into_iter().map(|line| line.text).collect();
                plain(&lines.join("\n"))
            }
            Payload::Times(times) => {
                let Some(id) = candidate.id else {
                    return Ok(None);
                };
                if texts.is_none() {
                    *texts = Some(self.search(title, query, KIND_TEXT).await?);
                }
                let text = texts
                    .as_deref()
                    .into_iter()
                    .flatten()
                    .filter(|text| text.id.as_deref() == Some(&*id))
                    .find_map(|text| match &text.lyrics {
                        Payload::Text(text) => Some(text.as_str()),
                        Payload::Lines(_) | Payload::Times(_) => None,
                    });
                match text {
                    Some(text) if can_trust_times => {
                        response::lined_up(text, &times).and_then(timed)
                    }
                    Some(text) => plain(text),
                    None => None,
                }
            }
        })
    }

    async fn search(
        &self,
        title: &str,
        query: &TrackQuery,
        kind: &str,
    ) -> Result<Vec<Candidate>, Error> {
        let form = Serializer::new(String::new())
            .append_pair("maxCount", MOST_CANDIDATES)
            .append_pair("key_title", title)
            .append_pair("key_artist", query.artist.as_deref().unwrap_or_default())
            .append_pair("terminalType", TERMINAL_TYPE)
            .append_pair("clientAppId", CLIENT_APP_ID)
            .append_pair("lyricsType", kind)
            .finish();
        let body = self.post(form).await?;
        response::candidates(&body)
    }

    async fn post(&self, form: String) -> Result<Bytes, Error> {
        let response = self.net.post_form(self.url.clone(), form).await?;
        if !response.status().is_success() {
            return Err(Error::HttpStatus(response.status().as_u16()));
        }
        Ok(response.into_body())
    }
}

impl Provider for Client {
    fn find_lyrics<'a>(&'a self, query: &'a TrackQuery) -> BoxFuture<'a, Result<Lyrics, Error>> {
        Client::find_lyrics(self, query).boxed()
    }
}

/// Which candidate to try first, least first. The right song without times beats
/// times for what may be another song.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Rank {
    has_other_title: bool,
    kind: KindRank,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum KindRank {
    TimesToTrust,
    Words,
    /// Its words may be right, for another version of the song.
    TimesOfAnotherLength,
}

impl Rank {
    fn of(candidate: &Candidate, query: &TrackQuery) -> Self {
        let offered = offered(candidate);
        let kind = match candidate.lyrics {
            Payload::Text(_) => KindRank::Words,
            Payload::Lines(_) | Payload::Times(_) if offered.is_another_length(query) => {
                KindRank::TimesOfAnotherLength
            }
            Payload::Lines(_) | Payload::Times(_) => KindRank::TimesToTrust,
        };
        Self {
            has_other_title: !offered.has_title_of(query),
            kind,
        }
    }
}

fn offered(candidate: &Candidate) -> Offered<'_> {
    Offered {
        title: candidate.title.as_deref(),
        artist: candidate.artist.as_deref(),
        length: candidate.length,
    }
}

fn plain(text: &str) -> Option<Lyrics> {
    let text = text.trim();
    (!text.is_empty()).then(|| Lyrics::Plain(PlainLyrics::new(text, None)))
}

fn timed(lines: Vec<Line>) -> Option<Lyrics> {
    let lines = timed_lines(lines.into_iter().map(raw_line).collect());
    (!lines.is_empty()).then(|| Lyrics::Timed(TimedLyrics::new(lines, None)))
}

fn raw_line(line: Line) -> RawLine {
    let start = line.start;
    let elements = line
        .words
        .into_iter()
        .map(|word| {
            let offset = word.start.and_then(|at| at.checked_sub(start));
            (Some(word.text), offset.map(|offset| offset.as_secs_f64()))
        })
        .collect();
    RawLine {
        start: Some(start.as_secs_f64()),
        end: line.end.map(|end| end.as_secs_f64()),
        text: Some(line.text),
        elements,
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead as _, BufReader, Read as _, Write as _};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use futures::executor::block_on;

    use super::response::fixture::{answer, song, song_of, xml_lyrics, xml_words};
    use super::*;

    fn service(answers: Vec<(u16, String)>) -> (Url, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        std::thread::spawn(move || {
            for (status, answer) in answers {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut length = 0;
                let mut is_form = false;
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    let lower = header.to_lowercase();
                    if let Some(value) = lower.strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                    is_form |= lower.contains("application/x-www-form-urlencoded");
                    if header.trim().is_empty() {
                        break;
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                assert!(is_form, "not sent as a form");
                let form = String::from_utf8(body).unwrap();
                recorded.lock().unwrap().push(form);
                write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                    answer.len(),
                )
                .unwrap();
            }
        });
        (url, requests)
    }

    fn client(url: Url) -> Client {
        Client {
            net: net::Client::new("hiphile-test").unwrap(),
            url,
        }
    }

    fn query() -> TrackQuery {
        TrackQuery::new("A Song").artist(Some("An Artist"))
    }

    fn of_200_seconds() -> TrackQuery {
        query().duration(Some(Duration::from_secs(200)))
    }

    fn find(query: &TrackQuery, answers: Vec<String>) -> Result<Lyrics, Error> {
        let (url, _) = service(answers.into_iter().map(|answer| (200, answer)).collect());
        block_on(client(url).find_lyrics(query))
    }

    fn line_texts(found: &Lyrics) -> Vec<String> {
        match found {
            Lyrics::Timed(timed) => timed
                .lines()
                .iter()
                .map(|line| line.text().to_string())
                .collect(),
            other => panic!("expected timed lyrics, got {other:?}"),
        }
    }

    fn plain_text(found: &Lyrics) -> String {
        match found {
            Lyrics::Plain(plain) => plain.text().to_string(),
            other => panic!("expected plain lyrics, got {other:?}"),
        }
    }

    #[test]
    fn a_search_is_sent_as_an_encoded_form() {
        let (url, requests) = service(vec![(200, answer(&[]))]);
        let query = TrackQuery::new("夜 & 朝+").artist(Some("A B"));

        block_on(client(url).find_lyrics(&query)).unwrap();

        let requests = requests.lock().unwrap();
        let pairs: Vec<(String, String)> = url::form_urlencoded::parse(requests[0].as_bytes())
            .into_owned()
            .collect();
        let pair = |key: &str, value: &str| (key.to_owned(), value.to_owned());
        assert_eq!(
            pairs,
            [
                pair("maxCount", "10"),
                pair("key_title", "夜 & 朝+"),
                pair("key_artist", "A B"),
                pair("terminalType", "10"),
                pair("clientAppId", "p1232089"),
                pair("lyricsType", "3"),
            ]
        );
    }

    #[test]
    fn an_unknown_artist_is_sent_empty_and_leaves_it_to_the_length() {
        let by_anyone = |millis| answer(&[song_of("1", "A Song", "Anyone", millis, "1", b"words")]);
        let query = TrackQuery::new("A Song").duration(Some(Duration::from_secs(200)));

        let (url, requests) = service(vec![(200, by_anyone(203_000))]);
        let found = block_on(client(url).find_lyrics(&query)).unwrap();
        assert_eq!(plain_text(&found), "words");
        assert!(requests.lock().unwrap()[0].contains("key_artist=&"));

        for millis in [100_000, 0] {
            let found = find(&query, vec![by_anyone(millis)]).unwrap();
            assert_eq!(found, Lyrics::NotFound);
        }
    }

    #[test]
    fn failures_of_the_request_are_errors() {
        let (url, _) = service(vec![(500, String::new())]);
        let error = block_on(client(url).find_lyrics(&query())).unwrap_err();
        assert!(matches!(error, Error::HttpStatus(500)), "{error:?}");

        let refusal = "<response><status>1</status></response>".to_owned();
        let error = find(&query(), vec![refusal]).unwrap_err();
        assert!(matches!(error, Error::Api { .. }), "{error:?}");
    }

    #[test]
    fn nothing_listed_is_not_found() {
        assert_eq!(find(&query(), vec![answer(&[])]).unwrap(), Lyrics::NotFound);
    }

    #[test]
    fn text_is_trimmed_and_keeps_its_inner_lines() {
        let song = song("1", "A Song", "1", b"\n first\n\nsecond \n");
        let found = find(&query(), vec![answer(&[song])]).unwrap();
        assert_eq!(plain_text(&found), "first\n\nsecond");
    }

    fn parts(line: &crate::TimedLine) -> Vec<(&str, Duration)> {
        line.elements()
            .iter()
            .map(|element| (&line.text()[element.range()], element.start()))
            .collect()
    }

    #[test]
    fn words_become_parts_of_a_line_from_the_start_of_the_song() {
        let xml = xml_words(
            "a b",
            &[
                ("a", "1000", "1200"),
                (" ", "1200", "1200"),
                ("b", "1500", "1800"),
            ],
        );
        let found = find(&query(), vec![answer(&[song("1", "A Song", "3", &xml)])]).unwrap();
        let Lyrics::Timed(timed) = &found else {
            panic!("expected timed lyrics: {found:?}");
        };
        let line = &timed.lines()[0];

        assert_eq!(&**line.text(), "a b");
        assert_eq!(
            parts(line),
            [
                ("a", Duration::from_millis(1000)),
                (" ", Duration::from_millis(1200)),
                ("b", Duration::from_millis(1500)),
            ]
        );
        assert_eq!(line.end(), Some(Duration::from_millis(1800)));
    }

    #[test]
    fn a_word_with_an_unusable_time_joins_the_one_before() {
        let xml = xml_words(
            "abcd",
            &[
                ("a", "1000", "1100"),
                ("b", "x", "1200"),
                ("c", "900", "1300"),
                ("d", "1500", "1600"),
            ],
        );
        let found = find(&query(), vec![answer(&[song("1", "A Song", "3", &xml)])]).unwrap();
        let Lyrics::Timed(timed) = &found else {
            panic!("expected timed lyrics: {found:?}");
        };

        assert_eq!(
            parts(&timed.lines()[0]),
            [
                ("abc", Duration::from_millis(1000)),
                ("d", Duration::from_millis(1500)),
            ]
        );
    }

    #[test]
    fn times_of_another_length_drop_the_words_too() {
        let xml = xml_words("ab", &[("a", "1000", "1100"), ("b", "1100", "1200")]);
        let song = song_of("1", "A Song", "An Artist", 300_000, "3", &xml);
        let found = find(&of_200_seconds(), vec![answer(&[song])]).unwrap();
        assert_eq!(plain_text(&found), "ab");
    }

    #[test]
    fn lines_without_words_to_use_are_timed_as_a_whole() {
        let xml = xml_lyrics(&[("one", "1000"), ("two", "2000"), ("", "3000")]);
        let found = find(&query(), vec![answer(&[song("1", "A Song", "3", &xml)])]).unwrap();

        assert_eq!(line_texts(&found), ["one", "two"]);
        let Lyrics::Timed(timed) = &found else {
            unreachable!()
        };
        assert_eq!(timed.lines()[0].start(), Duration::from_secs(1));
        assert_eq!(timed.lines()[0].elements().len(), 1);
        assert_eq!(timed.lines()[1].end(), Some(Duration::from_secs(3)));
    }

    #[test]
    fn binary_times_take_their_words_from_the_same_record_of_a_second_request() {
        let times = lsy::payload(1, 0xccc2, &[100, 200, 300]);
        let (url, requests) = service(vec![
            (200, answer(&[song("7", "A Song", "2", &times)])),
            (
                200,
                answer(&[
                    song("8", "A Song", "1", b"wrong\nrecord\n"),
                    song("7", "A Song", "1", b"one\ntwo\n"),
                ]),
            ),
        ]);

        let found = block_on(client(url).find_lyrics(&query())).unwrap();

        assert_eq!(line_texts(&found), ["one", "two"]);
        let Lyrics::Timed(timed) = &found else {
            unreachable!()
        };
        // The blank last line is when the last line sung ends
        assert_eq!(timed.lines()[1].end(), Some(Duration::from_secs(3)));
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].ends_with("lyricsType=3"));
        assert!(requests[1].ends_with("lyricsType=1"));
    }

    #[test]
    fn one_answer_gives_the_words_of_all_binary_candidates() {
        let times = lsy::payload(0, 0, &[100, 200]);
        let (url, requests) = service(vec![
            (
                200,
                answer(&[
                    song("1", "A Song", "2", &times),
                    song("2", "A Song", "2", &times),
                ]),
            ),
            // There are no words for the first
            (200, answer(&[song("2", "A Song", "1", b"one\ntwo")])),
        ]);

        let found = block_on(client(url).find_lyrics(&query())).unwrap();

        assert_eq!(line_texts(&found), ["one", "two"]);
        assert_eq!(requests.lock().unwrap().len(), 2);
    }

    #[test]
    fn words_that_do_not_fit_the_times_are_turned_down() {
        let times = lsy::payload(0, 0, &[100, 200]);
        let found = find(
            &query(),
            vec![
                answer(&[song("1", "A Song", "2", &times)]),
                answer(&[song("1", "A Song", "1", b"one\ntwo\nthree")]),
            ],
        )
        .unwrap();
        assert_eq!(found, Lyrics::NotFound);
    }

    #[test]
    fn a_failure_of_the_second_request_is_an_error() {
        let times = lsy::payload(0, 0, &[100, 200]);
        let (url, _) = service(vec![
            (200, answer(&[song("1", "A Song", "2", &times)])),
            (500, String::new()),
        ]);
        let error = block_on(client(url).find_lyrics(&query())).unwrap_err();
        assert!(matches!(error, Error::HttpStatus(500)), "{error:?}");
    }

    #[test]
    fn times_are_preferred_to_words() {
        let xml = xml_lyrics(&[("timed", "1000")]);
        let found = find(
            &query(),
            vec![answer(&[
                song("1", "A Song", "1", b"words"),
                song("2", "A Song", "3", &xml),
            ])],
        )
        .unwrap();
        assert_eq!(line_texts(&found), ["timed"]);
    }

    #[test]
    fn another_song_is_not_taken_for_this_one() {
        let other_title = song("1", "A Song Of Another Name", "1", b"words");
        let other_artist = song_of("2", "A Song", "Someone Else", 0, "1", b"words");
        for song in [other_title, other_artist] {
            let found = find(&of_200_seconds(), vec![answer(&[song])]).unwrap();
            assert_eq!(found, Lyrics::NotFound);
        }
    }

    #[test]
    fn the_right_title_beats_times_for_another_title() {
        let lost = |title: &str, millis, kind, data: &[u8]| {
            song_of("1", title, "An Artist", millis, kind, data)
        };
        let query = TrackQuery::new("Lost")
            .artist(Some("An Artist"))
            .duration(Some(Duration::from_secs(240)));
        let times = xml_lyrics(&[("timed", "1000")]);
        // Close enough in length to pass for the song on its own
        let other_title = lost("Lost Children", 241_000, "3", &times);

        let words = lost("Lost", 0, "1", b"words");
        let found = find(&query, vec![answer(&[other_title.clone(), words])]).unwrap();
        assert_eq!(plain_text(&found), "words");

        let another_length = lost("Lost", 300_000, "3", &times);
        let found = find(&query, vec![answer(&[other_title, another_length])]).unwrap();
        assert_eq!(plain_text(&found), "timed");
    }

    #[test]
    fn a_song_by_someone_whose_name_merely_looks_alike_is_another_song() {
        let of_another_length =
            |artist: &str| answer(&[song_of("1", "A Song", artist, 210_000, "1", b"words")]);
        for (asked, offered) in [("AI", "Kaito"), ("Fire", "Firestarter")] {
            let query = TrackQuery::new("A Song")
                .artist(Some(asked))
                .duration(Some(Duration::from_secs(200)));
            let found = find(&query, vec![of_another_length(offered)]).unwrap();
            assert_eq!(found, Lyrics::NotFound, "{asked} and {offered}");
        }
    }

    #[test]
    fn an_artist_written_in_another_script_keeps_the_times_of_the_same_length() {
        let query = TrackQuery::new("A Song")
            .artist(Some("Kenshi Yonezu"))
            .duration(Some(Duration::from_secs(256)));
        let xml = xml_lyrics(&[("timed", "1000")]);
        let song = song_of("1", "A Song", "米津玄師", 255_000, "3", &xml);
        let found = find(&query, vec![answer(&[song])]).unwrap();
        assert_eq!(line_texts(&found), ["timed"]);
    }

    #[test]
    fn times_are_kept_for_a_length_the_service_has_some_way_off() {
        let query = TrackQuery::new("A Song")
            .artist(Some("An Artist"))
            .duration(Some(Duration::from_secs(261)));
        let xml = xml_lyrics(&[("timed", "1000")]);
        let song = song_of("1", "A Song", "An Artist", 275_000, "3", &xml);
        let found = find(&query, vec![answer(&[song])]).unwrap();
        assert_eq!(line_texts(&found), ["timed"]);
    }

    #[test]
    fn an_artist_of_symbols_is_not_taken_for_anyone_without_the_length() {
        let query = TrackQuery::new("A Song").artist(Some("★"));
        let song = song_of("1", "A Song", "Someone", 200_000, "1", b"words");
        assert_eq!(
            find(&query, vec![answer(&[song])]).unwrap(),
            Lyrics::NotFound
        );
    }

    #[test]
    fn a_title_of_symbols_is_not_held_by_every_title() {
        let query = TrackQuery::new("†")
            .artist(Some("An Artist"))
            .duration(Some(Duration::from_secs(200)));
        let song = song_of("1", "Whatever Else", "An Artist", 200_000, "1", b"words");
        assert_eq!(
            find(&query, vec![answer(&[song])]).unwrap(),
            Lyrics::NotFound
        );
    }

    #[test]
    fn a_song_found_under_one_of_its_artists_is_taken() {
        let query = TrackQuery::new("A Song")
            .artist(Some("Kaneee & STUTS"))
            .duration(Some(Duration::from_secs(181)));
        let song = song_of("1", "A Song", "Kaneee", 181_000, "1", b"words");
        let found = find(&query, vec![answer(&[song])]).unwrap();
        assert_eq!(plain_text(&found), "words");
    }

    #[test]
    fn a_title_of_another_version_is_the_song_if_the_lengths_agree() {
        let query = TrackQuery::new("A Song (feat. Someone)")
            .artist(Some("an artist"))
            .duration(Some(Duration::from_secs(200)));
        let of_length = |millis| {
            let song = song_of("1", "a song", "An Artist, Someone", millis, "1", b"words");
            answer(&[song])
        };

        // Each is asked for twice, with its version and without
        let twice = |millis| vec![of_length(millis), of_length(millis)];
        let found = find(&query, twice(203_000)).unwrap();
        assert_eq!(plain_text(&found), "words");
        let found = find(&query, twice(250_000)).unwrap();
        assert_eq!(found, Lyrics::NotFound);
        let found = find(&query, twice(0)).unwrap();
        assert_eq!(found, Lyrics::NotFound);
    }

    fn with_version() -> TrackQuery {
        TrackQuery::new("A Song (feat. Someone)")
            .artist(Some("An Artist"))
            .duration(Some(Duration::from_secs(200)))
    }

    fn words_with_version() -> String {
        answer(&[song("1", "A Song (feat.Someone)", "1", b"words")])
    }

    fn times_without_version() -> String {
        let xml = xml_lyrics(&[("timed", "1000")]);
        answer(&[song_of("2", "A Song", "An Artist", 200_000, "3", &xml)])
    }

    #[test]
    fn times_under_the_title_without_its_version_beat_words_under_it() {
        let (url, requests) = service(vec![
            (200, words_with_version()),
            (200, times_without_version()),
        ]);

        let found = block_on(client(url).find_lyrics(&with_version())).unwrap();

        assert_eq!(line_texts(&found), ["timed"]);
        let requests = requests.lock().unwrap();
        assert!(
            requests[0].contains("key_title=A+Song+%28feat"),
            "{requests:?}"
        );
        assert!(requests[1].contains("key_title=A+Song&"), "{requests:?}");
    }

    #[test]
    fn words_of_binary_times_come_from_the_title_the_times_came_under() {
        let times = lsy::payload(0, 0, &[100, 200]);
        let (url, requests) = service(vec![
            (200, answer(&[])),
            (
                200,
                answer(&[song_of("7", "A Song", "An Artist", 200_000, "2", &times)]),
            ),
            (200, answer(&[song("7", "A Song", "1", b"one\ntwo")])),
        ]);

        let found = block_on(client(url).find_lyrics(&with_version())).unwrap();

        assert_eq!(line_texts(&found), ["one", "two"]);
        let requests = requests.lock().unwrap();
        assert!(requests[2].contains("key_title=A+Song&"), "{requests:?}");
        assert!(requests[2].ends_with("lyricsType=1"), "{requests:?}");
    }

    #[test]
    fn words_are_kept_when_the_title_without_its_version_has_no_times() {
        let found = find(&with_version(), vec![words_with_version(), answer(&[])]).unwrap();
        assert_eq!(plain_text(&found), "words");

        let (url, _) = service(vec![(200, words_with_version()), (500, String::new())]);
        let found = block_on(client(url).find_lyrics(&with_version())).unwrap();
        assert_eq!(plain_text(&found), "words");
    }

    #[test]
    fn the_title_without_its_version_can_have_the_only_words() {
        let words = answer(&[song_of("1", "A Song", "An Artist", 200_000, "1", b"words")]);
        let found = find(&with_version(), vec![answer(&[]), words]).unwrap();
        assert_eq!(plain_text(&found), "words");

        let (url, _) = service(vec![(200, answer(&[])), (500, String::new())]);
        let error = block_on(client(url).find_lyrics(&with_version())).unwrap_err();
        assert!(matches!(error, Error::HttpStatus(500)), "{error:?}");
    }

    #[test]
    fn times_under_the_title_as_written_are_not_asked_about_again() {
        let xml = xml_lyrics(&[("timed", "1000")]);
        let song = song_of(
            "1",
            "A Song (feat. Someone)",
            "An Artist",
            200_000,
            "3",
            &xml,
        );
        let (url, requests) = service(vec![(200, answer(&[song]))]);

        let found = block_on(client(url).find_lyrics(&with_version())).unwrap();

        assert_eq!(line_texts(&found), ["timed"]);
        assert_eq!(requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn times_of_another_length_are_dropped_and_their_words_kept() {
        let xml = xml_lyrics(&[("one", "1000"), ("two", "2000")]);
        let song_with_lines = song_of("1", "A Song", "An Artist", 300_000, "3", &xml);
        let found = find(&of_200_seconds(), vec![answer(&[song_with_lines])]).unwrap();
        assert_eq!(plain_text(&found), "one\ntwo");

        let times = lsy::payload(0, 0, &[100, 200]);
        let found = find(
            &of_200_seconds(),
            vec![
                answer(&[song_of("2", "A Song", "An Artist", 300_000, "2", &times)]),
                answer(&[song("2", "A Song", "1", b"one\ntwo\nthree")]),
            ],
        )
        .unwrap();
        assert_eq!(plain_text(&found), "one\ntwo\nthree");
    }

    #[test]
    fn words_are_preferred_to_times_of_another_length() {
        let xml = xml_lyrics(&[("timed", "1000")]);
        let found = find(
            &of_200_seconds(),
            vec![answer(&[
                song_of("1", "A Song", "An Artist", 300_000, "3", &xml),
                song("2", "A Song", "1", b"words"),
            ])],
        )
        .unwrap();
        assert_eq!(plain_text(&found), "words");
    }
}
