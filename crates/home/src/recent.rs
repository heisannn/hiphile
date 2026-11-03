use subsonic::Song;

/// The `count` songs in `songs` played last, most recent first. Songs never
/// played are left out.
pub(crate) fn most_recent(mut songs: Vec<Song>, count: usize) -> Vec<Song> {
    songs.retain(|song| song.played().is_some());
    songs.sort_by(|a, b| b.played().map(played_key).cmp(&a.played().map(played_key)));
    songs.truncate(count);
    songs
}

/// Orders RFC 3339 times, whose fractions of a second vary in length, which
/// comparing the text gets wrong: `.25Z` is before `.253Z`. Assumes the server
/// writes every time in one offset, as it does.
fn played_key(played: &str) -> (&str, u32) {
    const SECONDS_LEN: usize = "2026-01-01T00:00:00".len();
    let seconds = played.get(..SECONDS_LEN).unwrap_or(played);
    let fraction = played
        .get(SECONDS_LEN..)
        .and_then(|rest| rest.strip_prefix('.'));
    let nanos = fraction.map_or(0, |fraction| {
        let digits = fraction.bytes().take_while(u8::is_ascii_digit).take(9);
        let (value, len) = digits.fold((0, 0), |(value, len), digit| {
            (value * 10 + u32::from(digit - b'0'), len + 1)
        });
        value * 10_u32.pow(9 - len)
    });
    (seconds, nanos)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn song(id: &str, played: Option<&str>) -> Song {
        serde_json::from_value(json!({ "id": id, "title": id, "played": played })).unwrap()
    }

    fn ids(songs: &[Song]) -> Vec<&str> {
        songs.iter().map(|song| song.id().as_str()).collect()
    }

    #[test]
    fn newest_first_and_only_as_many_as_asked_for() {
        let songs = vec![
            song("old", Some("2026-07-01T10:00:00Z")),
            song("new", Some("2026-11-03T22:36:15Z")),
            song("middle", Some("2026-07-24T14:24:06Z")),
        ];
        assert_eq!(ids(&most_recent(songs, 2)), ["new", "middle"]);
    }

    #[test]
    fn songs_never_played_are_left_out() {
        let songs = vec![
            song("never", None),
            song("once", Some("2026-07-01T10:00:00Z")),
        ];
        assert_eq!(ids(&most_recent(songs, 8)), ["once"]);
    }

    #[test]
    fn fractions_of_different_lengths_compare_by_value() {
        let songs = vec![
            song("250ms", Some("2026-11-03T22:36:15.25Z")),
            song("253ms", Some("2026-11-03T22:36:15.253Z")),
            song("0ms", Some("2026-11-03T22:36:15Z")),
            song("nanos", Some("2026-11-03T22:36:15.250000001Z")),
        ];
        assert_eq!(
            ids(&most_recent(songs, 8)),
            ["253ms", "nanos", "250ms", "0ms"]
        );
    }

    #[test]
    fn text_that_is_not_a_time_does_not_panic() {
        let songs = vec![song("short", Some("yesterday")), song("empty", Some(""))];
        assert_eq!(most_recent(songs, 8).len(), 2);
    }
}
