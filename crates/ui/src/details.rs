use gpui_kit::SharedString;

/// The parts there are of a line of details about something, in order.
pub fn join_details<S: AsRef<str>>(parts: impl IntoIterator<Item = Option<S>>) -> SharedString {
    let parts: Vec<S> = parts.into_iter().flatten().collect();
    let parts: Vec<&str> = parts.iter().map(AsRef::as_ref).collect();
    parts.join(" · ").into()
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

#[cfg(test)]
mod tests {
    use super::*;

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
