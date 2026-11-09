use serde_json::json;

use super::*;
use crate::Lyrics;

fn secs(seconds: f64) -> Duration {
    Duration::from_secs_f64(seconds)
}

/// Lyrics of `lines` lines, five seconds apart from ten seconds in.
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
    let body = serde_json::Value::Array(body).to_string();
    crate::lyrics_from_richsync_body(&body)
}

fn first_timed_line(found: &Lyrics) -> &TimedLine {
    let Lyrics::Timed(found) = found else {
        panic!("expected timed lyrics");
    };
    &found.lines()[0]
}

#[test]
fn a_part_of_a_line_lights_as_it_starts_and_dims_as_the_line_ends() {
    let found = lyrics(2);
    let line = first_timed_line(&found);
    let second = line.elements()[1].start();
    assert_eq!(second, secs(12.));
    let end = line.end().unwrap();
    assert_eq!(end, secs(15.));

    assert_eq!(emphasis(line, second, secs(11.), true), 0.);
    assert_eq!(emphasis(line, second, secs(12.), true), 0.);
    assert!((emphasis(line, second, second + ELEMENT_FADE / 2, true) - 0.5).abs() < 1e-3);
    assert_eq!(emphasis(line, second, secs(13.), true), 1.);
    assert!((emphasis(line, second, end + LINE_FADE / 2, true) - 0.5).abs() < 1e-3);
    assert_eq!(emphasis(line, second, end + LINE_FADE, true), 0.);
}

#[test]
fn a_paused_song_holds_no_part_of_a_line_partway() {
    let found = lyrics(2);
    let line = first_timed_line(&found);
    let second = line.elements()[1].start();
    let end = line.end().unwrap();

    assert_eq!(emphasis(line, second, second - secs(0.1), false), 0.);
    assert_eq!(emphasis(line, second, second, false), 1.);
    assert_eq!(emphasis(line, second, end + LINE_FADE / 2, false), 0.);
}

#[test]
fn a_line_chosen_lights_though_seeking_lands_just_before_it() {
    let found = lyrics(3);
    let Lyrics::Timed(lyrics) = &found else {
        panic!("timed lyrics");
    };
    let start = lyrics.lines()[1].start();
    let landed = start - Duration::from_micros(20);
    assert_eq!(
        Lit::of(lyrics, landed, LyricsHighlight::ByWord).line,
        Some(1)
    );
    let lit = lit_lines(lyrics, landed, true, LyricsHighlight::ByLine);
    assert_eq!(lit.lines().last().map(LitLine::ix), Some(1));
}

fn timed(found: &Lyrics) -> &TimedLyrics {
    let Lyrics::Timed(found) = found else {
        panic!("expected timed lyrics");
    };
    found
}

/// The starts and ends of the parts lit in the line at `ix`.
fn lit_parts(lit: &LitLines, ix: usize) -> Vec<(usize, usize)> {
    let parts = lit.line(ix).map(LitLine::parts).unwrap_or_default();
    parts
        .iter()
        .map(|part| (part.range().start, part.range().end))
        .collect()
}

#[test]
fn only_the_parts_begun_are_lit_and_fading_is_noticed() {
    let found = lyrics(2);
    let found = timed(&found);
    let lit = |seconds| lit_lines(found, secs(seconds), true, LyricsHighlight::ByWord);

    let before = lit(9.);
    assert!(before.lines().is_empty() && !before.is_fading());

    let first = lit(11.);
    assert_eq!(lit_parts(&first, 0), [(0, "line ".len())]);
    assert_eq!(first.lines()[0].parts()[0].emphasis(), 1.);
    assert!(!first.is_fading());

    let second = lit(12.06);
    assert_eq!(
        lit_parts(&second, 0),
        [(0, "line ".len()), ("line ".len(), "line 0".len())]
    );
    assert!(second.is_fading());
}

#[test]
fn by_line_the_whole_line_lights_as_one() {
    let found = lyrics(2);
    let found = timed(&found);
    let lit = |seconds| lit_lines(found, secs(seconds), true, LyricsHighlight::ByLine);

    let before = lit(9.);
    assert!(before.lines().is_empty() && !before.is_fading());

    // The whole line, while the second part has yet to start
    let first = lit(11.);
    assert_eq!(lit_parts(&first, 0), [(0, "line 0".len())]);
    assert!(!first.is_fading());

    // A part starting is nothing to fade for
    let starting = lit(12.05);
    assert_eq!(starting.lines().len(), 1);
    assert!(!starting.is_fading());
}

#[test]
fn a_line_that_ended_fades_out_beside_the_next_then_is_left_out() {
    let found = lyrics(3);
    let found = timed(&found);
    let end = found.lines()[0].end().unwrap();

    let fading = lit_lines(found, end + LINE_FADE / 2, true, LyricsHighlight::ByWord);
    let lines: Vec<_> = fading.lines().iter().map(LitLine::ix).collect();
    assert_eq!(lines, [0, 1]);
    assert!(fading.is_fading());

    let faded = lit_lines(found, end + LINE_FADE, true, LyricsHighlight::ByWord);
    let lines: Vec<_> = faded.lines().iter().map(LitLine::ix).collect();
    assert_eq!(lines, [1]);

    let paused = lit_lines(found, end + LINE_FADE / 2, false, LyricsHighlight::ByWord);
    let lines: Vec<_> = paused.lines().iter().map(LitLine::ix).collect();
    assert_eq!(lines, [1]);
    assert!(!paused.is_fading());
}

#[test]
fn what_is_lit_changes_with_each_part_and_end() {
    let found = lyrics(2);
    let Lyrics::Timed(found) = &found else {
        unreachable!()
    };
    let lit = |seconds| Lit::of(found, secs(seconds), LyricsHighlight::ByWord);

    assert_eq!(lit(5.), Lit::default());
    assert_eq!(lit(10.), lit(11.));
    assert_ne!(lit(11.), lit(12.));
    assert_eq!(
        lit(14.),
        Lit {
            line: Some(0),
            elements: 2,
            has_ended: false
        }
    );
    assert_eq!(
        lit(15.),
        Lit {
            line: Some(1),
            elements: 1,
            has_ended: false
        }
    );
    assert_eq!(
        lit(19.5),
        Lit {
            line: Some(1),
            elements: 2,
            has_ended: true
        }
    );
}

#[test]
fn by_line_a_part_starting_is_no_cause_to_redraw() {
    let found = lyrics(2);
    let Lyrics::Timed(found) = &found else {
        panic!("timed lyrics");
    };
    let lit = |seconds, highlight| Lit::of(found, secs(seconds), highlight);

    assert_ne!(
        lit(11., LyricsHighlight::ByWord),
        lit(12.5, LyricsHighlight::ByWord)
    );
    assert_eq!(
        lit(11., LyricsHighlight::ByLine),
        lit(12.5, LyricsHighlight::ByLine)
    );
    assert_ne!(
        lit(11., LyricsHighlight::ByLine),
        lit(16., LyricsHighlight::ByLine)
    );
}
