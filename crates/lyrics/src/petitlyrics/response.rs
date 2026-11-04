//! Reads what the service answers: XML that lists candidates, each with its
//! lyrics in Base64. One unreadable candidate does not cost the others.

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use roxmltree::{Document, Node};

use super::lsy;
use crate::Error;

const SUCCESS: &str = "00000000";

/// A song the service offers lyrics for.
#[derive(Debug, PartialEq)]
pub(super) struct Candidate {
    pub(super) id: Option<String>,
    pub(super) title: Option<String>,
    pub(super) artist: Option<String>,
    pub(super) length: Option<Duration>,
    pub(super) lyrics: Payload,
}

#[derive(Debug, PartialEq)]
pub(super) enum Payload {
    /// As sent, which matters for lining it up with times.
    Text(String),
    /// Lines with their times, in order. A blank one is not sung, and says when
    /// the line before it ends.
    Lines(Vec<Line>),
    /// When each line starts, whose words come in another request.
    Times(Vec<Duration>),
}

#[derive(Debug, PartialEq)]
pub(super) struct Line {
    pub(super) start: Duration,
    /// When its last word ends, if that is known.
    pub(super) end: Option<Duration>,
    pub(super) text: String,
    /// Which together make `text`, or none if the line is timed only as a whole.
    pub(super) words: Vec<Word>,
}

impl Line {
    fn whole(start: Duration, text: String) -> Self {
        Self {
            start,
            end: None,
            text,
            words: Vec::new(),
        }
    }
}

/// A part of a line, often a single character, despite the service's name for it.
#[derive(Debug, PartialEq)]
pub(super) struct Word {
    pub(super) text: String,
    /// From the start of the song, if it can be read.
    pub(super) start: Option<Duration>,
}

/// The readable candidates in an answer. An answer that fails as a whole gives
/// none, even of those read by then.
pub(super) fn candidates(body: &[u8]) -> Result<Vec<Candidate>, Error> {
    let text = std::str::from_utf8(body).map_err(|_| Error::InvalidResponse)?;
    let document = Document::parse(text).map_err(|_| Error::InvalidResponse)?;
    let root = document.root_element();
    let status = child_text(root, "status").unwrap_or_default().trim();
    if status != SUCCESS {
        return Err(Error::Api {
            status: status.to_owned(),
            hint: None,
        });
    }
    Ok(children(root, "songs")
        .flat_map(|songs| children(songs, "song"))
        .filter_map(candidate)
        .collect())
}

fn candidate(song: Node<'_, '_>) -> Option<Candidate> {
    let kind = child_text(song, "lyricsType")?.trim();
    let data: Vec<u8> = child_text(song, "lyricsData")?
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    if data.is_empty() {
        return None;
    }
    let bytes = STANDARD.decode(data).ok()?;
    let length = child_text(song, "duration")
        .and_then(|millis| millis.trim().parse::<u64>().ok())
        .filter(|millis| *millis > 0)
        .map(Duration::from_millis);
    let lyrics = match kind {
        "1" => Payload::Text(
            String::from_utf8(bytes)
                .ok()
                .filter(|text| !text.trim().is_empty())?,
        ),
        "2" => Payload::Times(lsy::times(&bytes, length)?),
        "3" => Payload::Lines(lines(&bytes)?),
        _ => return None,
    };
    let detail = |name| {
        child_text(song, name)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    Some(Candidate {
        id: detail("lyricsId"),
        title: detail("title"),
        artist: detail("artist"),
        length,
        lyrics,
    })
}

/// The lines of lyrics in XML, in order of their times, unless it can't be read or
/// has none with words. A line's time is that of its first word, in milliseconds
/// from the start of the song, and zero if that can't be read.
fn lines(xml: &[u8]) -> Option<Vec<Line>> {
    let document = Document::parse(std::str::from_utf8(xml).ok()?).ok()?;
    let mut lines: Vec<Line> = Vec::new();
    for line in document
        .root_element()
        .descendants()
        .filter(|node| is_named(*node, "line"))
    {
        let text = child_text(line, "linestring").unwrap_or_default().trim();
        let time = children(line, "word")
            .next()
            .and_then(|word| child_text(word, "starttime"))
            .and_then(|time| time.trim().parse::<i64>().ok());
        let time = match time {
            Some(time) => time,
            // A blank line is only of use for its time.
            None if text.is_empty() => continue,
            None => 0,
        };
        // A negative time leaves the line out.
        if let Ok(millis) = u64::try_from(time) {
            lines.push(Line {
                end: children(line, "word")
                    .filter_map(|word| time_in(word, "endtime"))
                    .max(),
                words: words_of(line, text),
                ..Line::whole(Duration::from_millis(millis), text.to_owned())
            });
        }
    }
    lines.sort_by_key(|line| line.start);
    lines
        .iter()
        .any(|line| !line.text.is_empty())
        .then_some(lines)
}

/// The words of `line` if they make its `text`, which is all there is to tell they
/// can be used, as the form of a word is not documented. Space around them goes,
/// as it does from the text.
fn words_of(line: Node<'_, '_>, text: &str) -> Vec<Word> {
    let mut words: Vec<Word> = children(line, "word")
        .filter_map(|word| {
            Some(Word {
                text: child_text(word, "wordstring")?.to_owned(),
                start: time_in(word, "starttime"),
            })
        })
        .collect();
    while words.last().is_some_and(|word| word.text.trim().is_empty()) {
        words.pop();
    }
    let leading = words
        .iter()
        .take_while(|word| word.text.trim().is_empty())
        .count();
    words.drain(..leading);
    if let Some(first) = words.first_mut() {
        first.text = first.text.trim_start().to_owned();
    }
    if let Some(last) = words.last_mut() {
        last.text = last.text.trim_end().to_owned();
    }
    let joined: String = words.iter().map(|word| word.text.as_str()).collect();
    if joined == text { words } else { Vec::new() }
}

/// A time in milliseconds held by the child of `node` called `name`.
fn time_in(node: Node<'_, '_>, name: &'static str) -> Option<Duration> {
    let millis = child_text(node, name)?.trim().parse::<u64>().ok()?;
    Some(Duration::from_millis(millis))
}

/// Lines `raw` up with `times`. Its blank lines count, and a time may belong to a
/// blank last line. Only surplus blank lines at the end are let go.
pub(super) fn lined_up(raw: &str, times: &[Duration]) -> Option<Vec<Line>> {
    let text = raw.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines: Vec<&str> = text.split('\n').collect();
    while lines.len() > times.len() && lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    (lines.len() == times.len()).then(|| {
        times
            .iter()
            .copied()
            .zip(lines)
            .map(|(start, text)| Line::whole(start, text.to_owned()))
            .collect()
    })
}

/// Matches by local name, so any namespace will do.
fn is_named(node: Node<'_, '_>, name: &str) -> bool {
    node.is_element() && node.tag_name().name() == name
}

fn children<'a, 'input>(
    node: Node<'a, 'input>,
    name: &'static str,
) -> impl Iterator<Item = Node<'a, 'input>> {
    node.children().filter(move |child| is_named(*child, name))
}

fn child_text<'a>(node: Node<'a, '_>, name: &'static str) -> Option<&'a str> {
    children(node, name).next().and_then(|child| child.text())
}

/// Made-up answers, for tests here and of what asks for them.
#[cfg(test)]
pub(super) mod fixture {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;

    /// A `song` by the artist tests ask for, of no known length.
    pub(in super::super) fn song(id: &str, title: &str, kind: &str, data: &[u8]) -> String {
        song_of(id, title, "An Artist", 0, kind, data)
    }

    pub(in super::super) fn song_of(
        id: &str,
        title: &str,
        artist: &str,
        duration_ms: u64,
        kind: &str,
        data: &[u8],
    ) -> String {
        format!(
            "<song><lyricsId>{id}</lyricsId><title>{title}</title><artist>{artist}</artist>\
             <duration>{duration_ms}</duration><lyricsType>{kind}</lyricsType>\
             <lyricsData>{}</lyricsData></song>",
            STANDARD.encode(data),
        )
    }

    pub(in super::super) fn answer(songs: &[String]) -> String {
        format!(
            "<response><status>00000000</status><songs>{}</songs></response>",
            songs.concat(),
        )
    }

    /// Lyrics as XML, from each line's text and first word's time.
    pub(in super::super) fn xml_lyrics(lines: &[(&str, &str)]) -> Vec<u8> {
        let lines: String = lines
            .iter()
            .map(|(text, time)| {
                format!(
                    "<line><linestring>{text}</linestring>\
                     <word><starttime>{time}</starttime></word>\
                     <word><starttime>999999</starttime></word></line>"
                )
            })
            .collect();
        format!("<wsy>{lines}</wsy>").into_bytes()
    }

    /// Lyrics as XML of one line of `text`, from each word's text and times.
    pub(in super::super) fn xml_words(text: &str, words: &[(&str, &str, &str)]) -> Vec<u8> {
        let words: String = words
            .iter()
            .map(|(word, start, end)| {
                format!(
                    "<word><starttime>{start}</starttime><endtime>{end}</endtime>\
                     <wordstring>{word}</wordstring></word>"
                )
            })
            .collect();
        format!("<wsy><line><linestring>{text}</linestring>{words}</line></wsy>").into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::{answer, song, song_of, xml_lyrics, xml_words};
    use super::*;

    fn millis(time: u64) -> Duration {
        Duration::from_millis(time)
    }

    /// When each line of `candidate` starts and what it says.
    fn starts_and_texts(candidate: &Candidate) -> Vec<(Duration, &str)> {
        let Payload::Lines(lines) = &candidate.lyrics else {
            panic!("expected lines: {candidate:?}");
        };
        lines
            .iter()
            .map(|line| (line.start, line.text.as_str()))
            .collect()
    }

    /// The words of the one line in lyrics made from `xml`.
    fn words_in(xml: &[u8]) -> Vec<Word> {
        let found = one(&answer(&[song("1", "t", "3", xml)]));
        let Payload::Lines(mut lines) = found.lyrics else {
            panic!("expected lines");
        };
        assert_eq!(lines.len(), 1);
        lines.remove(0).words
    }

    fn word(text: &str, start: u64) -> Word {
        Word {
            text: text.to_owned(),
            start: Some(millis(start)),
        }
    }

    fn one(body: &str) -> Candidate {
        let mut found = candidates(body.as_bytes()).unwrap();
        assert_eq!(found.len(), 1, "{found:?}");
        found.remove(0)
    }

    #[test]
    fn a_status_other_than_success_is_an_error_whatever_else_is_there() {
        let good = song("1", "t", "1", b"words");
        for status in ["<status>00000001</status>", "<status></status>", ""] {
            let body = format!("<response>{status}<songs>{good}</songs></response>");
            let error = candidates(body.as_bytes()).unwrap_err();
            assert!(matches!(error, Error::Api { .. }), "{error:?}");
        }
    }

    #[test]
    fn xml_that_cant_be_read_is_an_error() {
        for body in ["<response><status>", "not xml", ""] {
            let error = candidates(body.as_bytes()).unwrap_err();
            assert!(matches!(error, Error::InvalidResponse), "{error:?}");
        }
        let error = candidates(&[0xff, 0xfe]).unwrap_err();
        assert!(matches!(error, Error::InvalidResponse), "{error:?}");
    }

    #[test]
    fn success_with_nothing_listed_is_no_candidates() {
        assert!(candidates(answer(&[]).as_bytes()).unwrap().is_empty());
        let body = "<response><status>00000000</status></response>";
        assert!(candidates(body.as_bytes()).unwrap().is_empty());
    }

    #[test]
    fn details_are_read_in_any_order_and_namespace_and_unknown_elements_are_skipped() {
        let data = STANDARD.encode("words");
        let body = format!(
            "<a:r xmlns:a='urn:x'><a:songs><a:song>\
             <a:unknown>x</a:unknown><a:lyricsData>{data}</a:lyricsData>\
             <a:lyricsType>1</a:lyricsType><a:title>夜 &amp; 朝</a:title>\
             <a:artist><![CDATA[A & B]]></a:artist><a:lyricsId>7</a:lyricsId>\
             <a:duration>1500</a:duration>\
             </a:song></a:songs><a:status>00000000</a:status></a:r>"
        );
        let found = one(&body);

        assert_eq!(found.id.as_deref(), Some("7"));
        assert_eq!(found.title.as_deref(), Some("夜 & 朝"));
        assert_eq!(found.artist.as_deref(), Some("A & B"));
        assert_eq!(found.length, Some(millis(1500)));
    }

    #[test]
    fn a_length_of_zero_is_no_length() {
        let found = one(&answer(&[song("1", "t", "1", b"words")]));
        assert_eq!(found.length, None);
    }

    #[test]
    fn a_bad_candidate_does_not_cost_the_others() {
        let data_of = |id, data: &str| {
            format!(
                "<song><lyricsId>{id}</lyricsId><lyricsType>1</lyricsType>\
                 <lyricsData>{data}</lyricsData></song>"
            )
        };
        let body = answer(&[
            data_of("empty", ""),
            data_of("not base64", "!!!"),
            data_of("url-safe base64", "-_-_"),
            song("not utf-8", "t", "1", &[0xff]),
            song("blank", "t", "1", b" \n "),
            song("unknown kind", "t", "9", b"words"),
            song("bad xml", "t", "3", b"<wsy><line>"),
            song("no words in xml", "t", "3", b"<wsy></wsy>"),
            song("bad binary", "t", "2", b"not one"),
            song("good", "t", "1", b"words"),
        ]);
        let found = candidates(body.as_bytes()).unwrap();

        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].id.as_deref(), Some("good"));
    }

    #[test]
    fn base64_may_hold_line_breaks() {
        let data = STANDARD.encode("words");
        let (start, end) = data.split_at(3);
        let body = format!(
            "<response><status>00000000</status><songs><song><lyricsType>1</lyricsType>\
             <lyricsData>{start}\r\n {end}\n</lyricsData></song></songs></response>"
        );
        assert_eq!(one(&body).lyrics, Payload::Text("words".to_owned()));
    }

    #[test]
    fn lines_take_the_time_of_their_first_word_and_are_put_in_order() {
        let xml = xml_lyrics(&[
            ("  later  ", "2000"),
            ("same time, first", "1000"),
            ("same time, second", "1000"),
        ]);
        let found = one(&answer(&[song("1", "t", "3", &xml)]));

        assert_eq!(
            starts_and_texts(&found),
            [
                (millis(1000), "same time, first"),
                (millis(1000), "same time, second"),
                (millis(2000), "later"),
            ]
        );
    }

    #[test]
    fn a_line_with_a_bad_time_starts_at_zero_and_a_negative_one_is_left_out() {
        let xml = b"<wsy><line><linestring>none</linestring></line>\
            <line><linestring>not a number</linestring>\
            <word><starttime>x</starttime></word></line>\
            <line><linestring>too big</linestring>\
            <word><starttime>99999999999999999999</starttime></word></line>\
            <line><linestring>negative</linestring>\
            <word><starttime>-5</starttime></word></line>\
            <line><linestring>fine</linestring>\
            <word><starttime>5</starttime></word></line></wsy>";
        let found = one(&answer(&[song("1", "t", "3", xml)]));

        assert_eq!(
            starts_and_texts(&found),
            [
                (millis(0), "none"),
                (millis(0), "not a number"),
                (millis(0), "too big"),
                (millis(5), "fine"),
            ]
        );
    }

    #[test]
    fn a_blank_line_with_a_time_stays_as_a_marker_and_without_one_goes() {
        let xml = b"<wsy><line><linestring> </linestring></line>\
            <line><linestring>sung</linestring><word><starttime>5</starttime></word></line>\
            <line><linestring></linestring><word><starttime>9</starttime></word></line></wsy>";
        let found = one(&answer(&[song("1", "t", "3", xml)]));

        assert_eq!(
            starts_and_texts(&found),
            [(millis(5), "sung"), (millis(9), "")]
        );
    }

    #[test]
    fn words_that_make_the_text_are_kept_with_their_times() {
        let xml = xml_words(
            "a b",
            &[("a", "10", "20"), (" ", "20", "20"), ("b", "30", "40")],
        );
        assert_eq!(
            words_in(&xml),
            [word("a", 10), word(" ", 20), word("b", 30)]
        );
    }

    #[test]
    fn a_line_ends_when_its_last_word_does() {
        let xml = xml_words("ab", &[("a", "10", "25"), ("b", "20", "x")]);
        let found = one(&answer(&[song("1", "t", "3", &xml)]));
        let Payload::Lines(lines) = found.lyrics else {
            panic!("expected lines");
        };
        assert_eq!(lines[0].start, millis(10));
        assert_eq!(lines[0].end, Some(millis(25)));
    }

    #[test]
    fn space_around_the_words_goes_as_it_does_from_the_text() {
        let xml = xml_words(
            " a b ",
            &[
                (" ", "1", "1"),
                (" a", "10", "20"),
                (" ", "20", "20"),
                ("b ", "30", "40"),
                (" ", "40", "40"),
            ],
        );
        assert_eq!(
            words_in(&xml),
            [word("a", 10), word(" ", 20), word("b", 30)]
        );
    }

    #[test]
    fn words_that_do_not_make_the_text_are_dropped_for_the_line_as_a_whole() {
        let xml = xml_words("ab", &[("a", "10", "20"), ("c", "20", "30")]);
        assert!(words_in(&xml).is_empty());
        let xml = xml_words("ab", &[("a", "10", "20")]);
        assert!(words_in(&xml).is_empty());
    }

    #[test]
    fn a_word_time_that_cant_be_read_is_none() {
        let xml = xml_words(
            "abc",
            &[("a", "10", "20"), ("b", "-5", "20"), ("c", "x", "20")],
        );
        let words = words_in(&xml);
        assert_eq!(words[0].start, Some(millis(10)));
        assert_eq!(words[1].start, None);
        assert_eq!(words[2].start, None);
    }

    #[test]
    fn only_blank_lines_are_no_lyrics() {
        let xml = xml_lyrics(&[("", "5"), (" ", "9")]);
        let body = answer(&[song("1", "t", "3", &xml)]);
        assert!(candidates(body.as_bytes()).unwrap().is_empty());
    }

    #[test]
    fn the_binary_form_gives_times_held_to_the_length_given() {
        let payload = lsy::payload(1, 0xccc2, &[100, 200]);
        let found = one(&answer(&[song("1", "t", "2", &payload)]));
        assert_eq!(
            found.lyrics,
            Payload::Times(vec![millis(1000), millis(2000)])
        );

        let past_it = song_of("1", "t", "a", 1500, "2", &payload);
        assert!(
            candidates(answer(&[past_it]).as_bytes())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn text_lines_up_with_times_blank_lines_and_all() {
        let times = [millis(1), millis(2), millis(3), millis(4)];
        let lined = lined_up("a\r\n\rb\n", &times).unwrap();
        let texts: Vec<&str> = lined.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(texts, ["a", "", "b", ""]);
        assert_eq!(lined[3].start, millis(4));
    }

    #[test]
    fn only_surplus_blank_lines_at_the_end_are_let_go() {
        let times = [millis(1), millis(2)];
        assert!(lined_up("a\nb\n\n \n", &times).is_some());
        assert!(lined_up("a\nb\nc", &times).is_none(), "too many");
        assert!(lined_up("a", &times).is_none(), "too few");
        assert!(lined_up("a\n\nb\n", &times).is_none(), "blank within");
    }
}
