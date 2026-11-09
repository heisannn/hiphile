//! How the lines of timed lyrics light up as a song goes on.

use std::ops::Range;
use std::time::Duration;

use config::LyricsHighlight;

use crate::{TimedLine, TimedLyrics};

const ELEMENT_FADE: Duration = Duration::from_millis(120);
const LINE_FADE: Duration = Duration::from_millis(180);
/// Added to where the song is for lighting the lines. Seeking lands on the frame
/// before the time asked for, and a line chosen should be the one that lights.
const SEEK_ROUNDING: Duration = Duration::from_millis(1);

/// Which line the song's position lights and how much of it, for telling when
/// that changes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Lit {
    pub line: Option<usize>,
    pub elements: usize,
    pub has_ended: bool,
}

impl Lit {
    pub fn of(lyrics: &TimedLyrics, position: Duration, highlight: LyricsHighlight) -> Self {
        let position = position + SEEK_ROUNDING;
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

/// The lines with a part lit at the song's position, in order: the current one and those still
/// fading out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LitLines {
    lines: Vec<LitLine>,
    is_fading: bool,
}

impl LitLines {
    pub fn lines(&self) -> &[LitLine] {
        &self.lines
    }

    /// The line at `ix` in the lyrics, if any part of it is lit.
    pub fn line(&self, ix: usize) -> Option<&LitLine> {
        self.lines.iter().find(|line| line.ix == ix)
    }

    /// Whether a part is between dim and lit, so that the next frame differs.
    pub fn is_fading(&self) -> bool {
        self.is_fading
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LitLine {
    ix: usize,
    parts: Vec<LitPart>,
}

impl LitLine {
    /// Where the line is in the lyrics.
    pub fn ix(&self) -> usize {
        self.ix
    }

    pub fn parts(&self) -> &[LitPart] {
        &self.parts
    }
}

/// A part of a line that is lit: by word an element, by line all of it.
#[derive(Clone, Debug, PartialEq)]
pub struct LitPart {
    range: Range<usize>,
    emphasis: f32,
}

impl LitPart {
    /// In bytes of the line's UTF-8 text.
    pub fn range(&self) -> Range<usize> {
        self.range.clone()
    }

    /// Above `0.0`, and `1.0` once fully lit.
    pub fn emphasis(&self) -> f32 {
        self.emphasis
    }
}

/// What the song's `position` lights. It looks only at the lines that can be lit, so it can run
/// every frame.
pub fn lit_lines(
    lyrics: &TimedLyrics,
    position: Duration,
    is_playing: bool,
    highlight: LyricsHighlight,
) -> LitLines {
    let position = position + SEEK_ROUNDING;
    let Some(current) = lyrics.line_at(position) else {
        return LitLines::default();
    };
    let mut lit = LitLines::default();
    for (ix, line) in lyrics.lines()[..=current].iter().enumerate().rev() {
        // A line ends no later than the next one does, so none before it is lit either
        if ix < current && line.end().is_some_and(|end| end + LINE_FADE <= position) {
            break;
        }
        let mut parts = Vec::new();
        let mut light = |range: Range<usize>, start: Duration| {
            let emphasis = emphasis(line, start, position, is_playing);
            if emphasis > 0. {
                lit.is_fading |= emphasis < 1.;
                parts.push(LitPart { range, emphasis });
            }
        };
        match highlight {
            LyricsHighlight::ByWord => {
                for element in line.elements() {
                    light(element.range(), element.start());
                }
            }
            LyricsHighlight::ByLine => light(0..line.text().len(), line.start()),
        }
        if !parts.is_empty() {
            lit.lines.push(LitLine { ix, parts });
        }
    }
    lit.lines.reverse();
    lit
}

/// How lit a part of a line starting at `start` is at `position`, from `0.0` to
/// `1.0`. It lights up as it starts and dims when its line ends, over time if the
/// song is playing. A song standing still would hold it partway.
fn emphasis(line: &TimedLine, start: Duration, position: Duration, is_playing: bool) -> f32 {
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
