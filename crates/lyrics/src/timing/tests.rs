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
    assert_eq!(lyrics.line_at(landed + SEEK_ROUNDING), Some(1));
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
