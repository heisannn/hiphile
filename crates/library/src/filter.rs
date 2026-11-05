use fuzzy::{Index, Speller};
use subsonic::Album;

pub(crate) fn album_index(albums: &[Album], speller: Option<&dyn Speller>) -> Index {
    Index::new(
        albums.iter().map(|album| {
            let name = Some(album.name());
            [name, album.artist()].into_iter().flatten()
        }),
        speller,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index() -> Index {
        let albums: Vec<Album> = serde_json::from_str(
            r#"[
                {"id":"1","name":"Kind of Blue","artist":"Miles Davis"},
                {"id":"2","name":"Blue Train","artist":"John Coltrane"},
                {"id":"3","name":"Giant Steps","artist":"John Coltrane"},
                {"id":"4","name":"Untitled"}
            ]"#,
        )
        .unwrap();
        album_index(&albums, None)
    }

    #[test]
    fn empty_query_matches_everything() {
        assert_eq!(index().search("  "), [0, 1, 2, 3]);
    }

    #[test]
    fn matches_name_or_artist_ignoring_case_the_better_first() {
        assert_eq!(index().search("blue"), [1, 0]);
        assert_eq!(index().search("COLTRANE"), [1, 2]);
    }

    #[test]
    fn albums_without_artist_match_by_name() {
        assert_eq!(index().search("untitled"), [3]);
        assert!(index().search("nobody").is_empty());
    }
}
