//! How the lines of timed lyrics light up as a song goes on.

use std::time::Duration;

use config::LyricsHighlight;

use crate::{TimedLine, TimedLyrics};

pub const ELEMENT_FADE: Duration = Duration::from_millis(120);
pub const LINE_FADE: Duration = Duration::from_millis(180);
/// Added to where the song is for lighting the lines. Seeking lands on the frame
/// before the time asked for, and a line chosen should be the one that lights.
pub const SEEK_ROUNDING: Duration = Duration::from_millis(1);

/// Which line a position lights and how much of it, for telling when that
/// changes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Lit {
    pub line: Option<usize>,
    pub elements: usize,
    pub has_ended: bool,
}

impl Lit {
    pub fn of(lyrics: &TimedLyrics, position: Duration, highlight: LyricsHighlight) -> Self {
        let Some(ix) = lyrics.line_at(position) else {
            return Self::default();
        };
        let line = &lyrics.lines()[ix];
        Self {
            line: Some(ix),
            elements: match highlight {
                LyricsHighlight::ByWord => line
                    .elements()
                    .partition_point(|element| element.start() <= position),
                LyricsHighlight::ByLine => 0,
            },
            has_ended: line.end().is_some_and(|end| end <= position),
        }
    }
}

/// How lit a part of a line starting at `start` is at `position`, from `0.0` to
/// `1.0`. It lights up as it starts and dims when its line ends, over time if the
/// song is playing. A song standing still would hold it partway.
pub fn emphasis(line: &TimedLine, start: Duration, position: Duration, is_playing: bool) -> f32 {
    let Some(lit_for) = position.checked_sub(start) else {
        return 0.;
    };
    let ended_for = line.end().and_then(|end| position.checked_sub(end));
    if !is_playing {
        return if ended_for.is_some() { 0. } else { 1. };
    }
    let rise = (lit_for.as_secs_f32() / ELEMENT_FADE.as_secs_f32()).min(1.);
    let fall = ended_for.map_or(0., |ended_for| {
        (ended_for.as_secs_f32() / LINE_FADE.as_secs_f32()).min(1.)
    });
    rise * (1. - fall)
}

#[cfg(test)]
mod tests;
