use std::time::Duration;

/// `m:ss`, or `h:mm:ss` from one hour.
pub fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

pub fn format_song_count(count: usize) -> String {
    match count {
        1 => "1 song".to_owned(),
        count => format!("{count} songs"),
    }
}

pub fn format_album_count(count: usize) -> String {
    match count {
        1 => "1 album".to_owned(),
        count => format!("{count} albums"),
    }
}

pub fn join_details<S: AsRef<str>>(parts: impl IntoIterator<Item = Option<S>>) -> String {
    let parts: Vec<S> = parts.into_iter().flatten().collect();
    let parts: Vec<&str> = parts.iter().map(AsRef::as_ref).collect();
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_like_a_music_player() {
        assert_eq!(format_duration(Duration::ZERO), "0:00");
        assert_eq!(format_duration(Duration::from_millis(65_900)), "1:05");
        assert_eq!(format_duration(Duration::from_secs(3_599)), "59:59");
        assert_eq!(format_duration(Duration::from_secs(3_600 + 62)), "1:01:02");
    }

    #[test]
    fn only_the_parts_there_are_join() {
        assert_eq!(join_details([Some("a"), None, Some("b")]), "a · b");
        assert_eq!(join_details([None, Some("a")]), "a");
        assert_eq!(join_details::<&str>([None]), "");
    }

    #[test]
    fn one_song_is_singular() {
        assert_eq!(format_song_count(0), "0 songs");
        assert_eq!(format_song_count(1), "1 song");
        assert_eq!(format_song_count(2), "2 songs");
        assert_eq!(format_album_count(1), "1 album");
        assert_eq!(format_album_count(2), "2 albums");
    }
}
