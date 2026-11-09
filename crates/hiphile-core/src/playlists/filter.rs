use fuzzy::{Index, Speller};
use subsonic::Playlist;

pub(super) fn playlist_index(playlists: &[Playlist], speller: Option<&dyn Speller>) -> Index {
    Index::new(
        playlists.iter().map(|playlist| {
            let name = Some(playlist.name());
            [name, playlist.owner()].into_iter().flatten()
        }),
        speller,
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn index() -> Index {
        let playlists: Vec<Playlist> = serde_json::from_value(json!([
            { "id": "1", "name": "Late Night Drive", "owner": "bob" },
            { "id": "2", "name": "Focus", "owner": "alice" },
            { "id": "3", "name": "Unowned" },
        ]))
        .unwrap();
        playlist_index(&playlists, None)
    }

    #[test]
    fn an_empty_query_matches_every_playlist() {
        assert_eq!(index().search("  "), [0, 1, 2]);
    }

    #[test]
    fn matches_the_name_or_the_owner_ignoring_case() {
        assert_eq!(index().search("NIGHT"), [0]);
        assert_eq!(index().search("alice"), [1]);
        assert!(index().search("nothing").is_empty());
    }
}
