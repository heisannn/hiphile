use subsonic::Playlist;

/// Positions of the playlists whose name or owner contains `query`, ignoring
/// case, in their original order. An empty query matches every playlist.
pub(crate) fn matching_playlists(playlists: &[Playlist], query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    playlists
        .iter()
        .enumerate()
        .filter(|(_, playlist)| {
            query.is_empty()
                || playlist.name().to_lowercase().contains(&query)
                || playlist
                    .owner()
                    .is_some_and(|owner| owner.to_lowercase().contains(&query))
        })
        .map(|(ix, _)| ix)
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn playlists() -> Vec<Playlist> {
        serde_json::from_value(json!([
            { "id": "1", "name": "Late Night Drive", "owner": "bob" },
            { "id": "2", "name": "Focus", "owner": "alice" },
            { "id": "3", "name": "Unowned" },
        ]))
        .unwrap()
    }

    #[test]
    fn an_empty_query_matches_every_playlist() {
        assert_eq!(matching_playlists(&playlists(), "  "), [0, 1, 2]);
    }

    #[test]
    fn matches_the_name_or_the_owner_ignoring_case() {
        assert_eq!(matching_playlists(&playlists(), "NIGHT"), [0]);
        assert_eq!(matching_playlists(&playlists(), "alice"), [1]);
        assert!(matching_playlists(&playlists(), "nothing").is_empty());
    }
}
