use subsonic::Album;

/// Positions of the albums whose name or artist contains `query`, ignoring
/// case, in their original order. An empty query matches every album.
pub(crate) fn matching_albums(albums: &[Album], query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    albums
        .iter()
        .enumerate()
        .filter(|(_, album)| {
            query.is_empty()
                || album.name().to_lowercase().contains(&query)
                || album
                    .artist()
                    .is_some_and(|artist| artist.to_lowercase().contains(&query))
        })
        .map(|(ix, _)| ix)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn albums() -> Vec<Album> {
        let json = r#"[
            {"id":"1","name":"Kind of Blue","artist":"Miles Davis"},
            {"id":"2","name":"Blue Train","artist":"John Coltrane"},
            {"id":"3","name":"Giant Steps","artist":"John Coltrane"},
            {"id":"4","name":"Untitled"}
        ]"#;
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn empty_query_matches_everything() {
        assert_eq!(matching_albums(&albums(), "  "), [0, 1, 2, 3]);
    }

    #[test]
    fn matches_name_or_artist_ignoring_case() {
        assert_eq!(matching_albums(&albums(), "blue"), [0, 1]);
        assert_eq!(matching_albums(&albums(), "COLTRANE"), [1, 2]);
    }

    #[test]
    fn albums_without_artist_match_by_name() {
        assert_eq!(matching_albums(&albums(), "untitled"), [3]);
        assert!(matching_albums(&albums(), "nobody").is_empty());
    }
}
