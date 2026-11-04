//! A song's lyrics, whatever kind of them there are.

use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

/// How long a line stays current after it ends when another comes soon, so
/// that a pause for breath does not dim it. A longer gap is a break.
const BREAK: Duration = Duration::from_secs(4);

/// What was found for a song.
#[derive(Clone, Debug, PartialEq)]
pub enum Lyrics {
    /// With times, by the line or finer.
    Timed(TimedLyrics),
    Plain(PlainLyrics),
    Instrumental,
    /// There are lyrics, which the service does not let be shown.
    Restricted,
    NotFound,
}

/// Lyrics with no times.
#[derive(Clone, Debug, PartialEq)]
pub struct PlainLyrics {
    text: Arc<str>,
    copyright: Option<Arc<str>>,
}

impl PlainLyrics {
    pub(crate) fn new(text: &str, copyright: Option<Arc<str>>) -> Self {
        Self {
            text: text.into(),
            copyright,
        }
    }

    pub fn text(&self) -> &Arc<str> {
        &self.text
    }

    pub fn copyright(&self) -> Option<&Arc<str>> {
        self.copyright.as_ref()
    }
}

/// Lines in the order they are sung, which is never out of order in time.
#[derive(Clone, Debug, PartialEq)]
pub struct TimedLyrics {
    lines: Vec<TimedLine>,
    copyright: Option<Arc<str>>,
}

impl TimedLyrics {
    /// `lines` holds at least one line.
    pub(crate) fn new(lines: Vec<TimedLine>, copyright: Option<Arc<str>>) -> Self {
        Self { lines, copyright }
    }

    pub fn lines(&self) -> &[TimedLine] {
        &self.lines
    }

    pub fn copyright(&self) -> Option<&Arc<str>> {
        self.copyright.as_ref()
    }

    /// The index of the last line started by `position`, if one has.
    pub fn line_at(&self, position: Duration) -> Option<usize> {
        self.lines
            .partition_point(|line| line.start <= position)
            .checked_sub(1)
    }
}

/// A line and when its parts start, from the start of the song.
#[derive(Clone, Debug, PartialEq)]
pub struct TimedLine {
    text: Arc<str>,
    start: Duration,
    end: Option<Duration>,
    elements: Vec<TimedTextElement>,
}

impl TimedLine {
    pub fn text(&self) -> &Arc<str> {
        &self.text
    }

    pub fn start(&self) -> Duration {
        self.start
    }

    /// When it stops being current, or `None` for a last line that is not
    /// known to end.
    pub fn end(&self) -> Option<Duration> {
        self.end
    }

    /// In order, covering all of the text. A line timed only as a whole is one
    /// element. An element is not necessarily a word.
    pub fn elements(&self) -> &[TimedTextElement] {
        &self.elements
    }
}

/// A part of a line that starts at its own time.
#[derive(Clone, Debug, PartialEq)]
pub struct TimedTextElement {
    range: Range<usize>,
    start: Duration,
}

impl TimedTextElement {
    /// Where it lies in its line's text, in bytes.
    pub fn range(&self) -> Range<usize> {
        self.range.clone()
    }

    pub fn start(&self) -> Duration {
        self.start
    }
}

/// A line as a provider describes it, whose times are yet to be checked.
#[derive(Debug, Default)]
pub(crate) struct RawLine {
    /// Seconds from the start of the song.
    pub(crate) start: Option<f64>,
    /// Seconds from the start of the song.
    pub(crate) end: Option<f64>,
    pub(crate) text: Option<String>,
    /// Text and seconds from the start of the line.
    pub(crate) elements: Vec<(Option<String>, Option<f64>)>,
}

/// The lines that make sense of `raw`, or none. A line without text says when
/// the line before it ends, and a line with a time out of order or unusable is
/// left out, as is a part with such a time, whose text joins the part before.
pub(crate) fn timed_lines(raw: Vec<RawLine>) -> Vec<TimedLine> {
    let mut lines: Vec<TimedLine> = Vec::with_capacity(raw.len());
    for line in raw {
        let Some(start) = line.start.and_then(seconds) else {
            continue;
        };
        if lines.last().is_some_and(|last| start < last.start) {
            continue;
        }
        let end = line.end.and_then(seconds).filter(|end| *end > start);
        let (text, elements) = texts_and_times(line.text, line.elements, start);
        if text.trim().is_empty() {
            if let Some(last) = lines.last_mut()
                && last.end.is_none()
            {
                last.end = Some(start);
            }
            continue;
        }
        lines.push(TimedLine {
            text: text.into(),
            start,
            end,
            elements,
        });
    }
    settle_ends(&mut lines);
    lines
}

fn texts_and_times(
    text: Option<String>,
    raw: Vec<(Option<String>, Option<f64>)>,
    line_start: Duration,
) -> (String, Vec<TimedTextElement>) {
    let mut joined = String::new();
    let mut elements: Vec<TimedTextElement> = Vec::with_capacity(raw.len());
    for (part, offset) in raw {
        let Some(part) = part.filter(|part| !part.is_empty()) else {
            continue;
        };
        let at = joined.len();
        joined.push_str(&part);
        let start = offset
            .and_then(seconds)
            .and_then(|offset| line_start.checked_add(offset));
        let last_start = elements.last().map(TimedTextElement::start);
        match start.filter(|start| last_start.is_none_or(|last| *start >= last)) {
            Some(start) => elements.push(TimedTextElement {
                range: at..joined.len(),
                start,
            }),
            None => match elements.last_mut() {
                Some(last) => last.range.end = joined.len(),
                None => elements.push(TimedTextElement {
                    range: at..joined.len(),
                    start: line_start,
                }),
            },
        }
    }
    if elements.is_empty() {
        let joined = text.unwrap_or_default();
        let element = TimedTextElement {
            range: 0..joined.len(),
            start: line_start,
        };
        return (joined, vec![element]);
    }
    (joined, elements)
}

/// A line ends when the next starts, unless a break comes between them.
fn settle_ends(lines: &mut [TimedLine]) {
    let starts: Vec<Duration> = lines.iter().map(TimedLine::start).collect();
    for (line, next_start) in lines.iter_mut().zip(starts.into_iter().skip(1)) {
        line.end = match line.end {
            Some(end) if next_start.saturating_sub(end) >= BREAK => Some(end),
            _ => Some(next_start),
        };
    }
}

/// Seconds as a duration, unless they are negative or not a number.
fn seconds(seconds: f64) -> Option<Duration> {
    Duration::try_from_secs_f64(seconds).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(start: f64, end: Option<f64>, text: &str) -> RawLine {
        RawLine {
            start: Some(start),
            end,
            text: Some(text.to_owned()),
            elements: Vec::new(),
        }
    }

    fn element(text: &str, offset: f64) -> (Option<String>, Option<f64>) {
        (Some(text.to_owned()), Some(offset))
    }

    fn secs(seconds: f64) -> Duration {
        Duration::from_secs_f64(seconds)
    }

    /// Each part of a line's text and when it starts.
    fn parts(line: &TimedLine) -> Vec<(&str, Duration)> {
        line.elements()
            .iter()
            .map(|element| (&line.text()[element.range()], element.start()))
            .collect()
    }

    #[test]
    fn parts_start_relative_to_their_line_and_keep_their_spaces() {
        let lines = timed_lines(vec![RawLine {
            start: Some(10.),
            end: Some(12.),
            text: Some("ignored for its parts".to_owned()),
            elements: vec![element("sample ", 0.), element("line", 1.5)],
        }]);

        assert_eq!(&**lines[0].text(), "sample line");
        assert_eq!(
            parts(&lines[0]),
            [("sample ", secs(10.)), ("line", secs(11.5))]
        );
    }

    #[test]
    fn a_line_without_parts_is_one_element() {
        let lines = timed_lines(vec![line(3., None, "a line")]);
        assert_eq!(parts(&lines[0]), [("a line", secs(3.))]);
    }

    #[test]
    fn unusable_times_do_not_lose_text_or_panic() {
        let lines = timed_lines(vec![RawLine {
            start: Some(10.),
            end: Some(f64::NAN),
            text: None,
            elements: vec![
                element("a", -1.),
                element("b", f64::INFINITY),
                element("c", 2.),
                element("d", 1.),
                (Some("e".to_owned()), None),
            ],
        }]);

        assert_eq!(&**lines[0].text(), "abcde");
        assert_eq!(parts(&lines[0]), [("ab", secs(10.)), ("cde", secs(12.))]);
        assert_eq!(lines[0].end(), None);
    }

    #[test]
    fn lines_with_unusable_or_earlier_starts_are_left_out() {
        let lines = timed_lines(vec![
            line(5., None, "first"),
            line(f64::NAN, None, "no time"),
            line(-1., None, "negative"),
            line(2., None, "earlier"),
            line(6., None, "second"),
        ]);

        let texts: Vec<&str> = lines.iter().map(|line| &**line.text()).collect();
        assert_eq!(texts, ["first", "second"]);
    }

    #[test]
    fn a_line_stays_current_until_the_next_unless_a_break_comes_between() {
        let lines = timed_lines(vec![
            line(0., Some(2.), "close to the next"),
            line(3., Some(4.), "before a break"),
            line(20., Some(19.), "ends before it starts"),
            line(25., None, "last, not known to end"),
        ]);

        assert_eq!(lines[0].end(), Some(secs(3.)));
        assert_eq!(lines[1].end(), Some(secs(4.)));
        assert_eq!(lines[2].end(), Some(secs(25.)));
        assert_eq!(lines[3].end(), None);
    }

    #[test]
    fn a_line_without_text_ends_the_one_before() {
        let lines = timed_lines(vec![
            line(0., None, "sung"),
            line(10., None, ""),
            line(30., None, "sung later"),
        ]);

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].end(), Some(secs(10.)));
    }

    #[test]
    fn the_line_at_a_position_is_the_last_one_started() {
        let lyrics = TimedLyrics::new(
            timed_lines(vec![line(5., None, "a"), line(9., None, "b")]),
            None,
        );

        assert_eq!(lyrics.line_at(secs(4.9)), None);
        assert_eq!(lyrics.line_at(secs(5.)), Some(0));
        assert_eq!(lyrics.line_at(secs(8.9)), Some(0));
        assert_eq!(lyrics.line_at(secs(9.)), Some(1));
        assert_eq!(lyrics.line_at(secs(900.)), Some(1));
    }
}
