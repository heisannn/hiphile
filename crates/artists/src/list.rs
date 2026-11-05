use std::collections::HashMap;

use fuzzy::{Index, Speller};
use subsonic::{Album, Artist, ArtistId, CoverArtId, Song};

/// `artists`, to find by their name.
pub(crate) fn artist_index(artists: &[Artist], speller: Option<&dyn Speller>) -> Index {
    Index::new(artists.iter().map(|artist| [artist.name()]), speller)
}

/// The artists that `albums` name, by name, to show until the server says.
pub(crate) fn artists_of_albums(albums: &[Album]) -> Vec<Artist> {
    let mut found: HashMap<&ArtistId, (&str, Option<&CoverArtId>, u32)> = HashMap::new();
    for album in albums {
        let (Some(id), Some(name)) = (album.artist_id(), album.artist()) else {
            continue;
        };
        // Tags can name an artist with nothing but spaces: the server knows better.
        if name.trim().is_empty() {
            continue;
        }
        let artist = found.entry(id).or_insert((name, album.cover_art(), 0));
        artist.2 += 1;
    }
    let mut artists: Vec<Artist> = found
        .into_iter()
        .map(|(id, (name, cover, count))| {
            Artist::new(id.clone(), name.to_owned(), cover.cloned(), Some(count))
        })
        .collect();
    artists.sort_by_cached_key(|artist| (artist.name().to_lowercase(), artist.id().clone()));
    artists
}

/// `songs` album by album in track order, as the songs of an artist who has no
/// albums of their own show.
pub(crate) fn album_by_album(mut songs: Vec<Song>) -> Vec<Song> {
    songs.sort_by_cached_key(|song| {
        let album = song.album().unwrap_or_default().to_lowercase();
        (album, song.disc_number(), song.track())
    });
    songs
}

/// `albums` by the artist, newest first; those without a year come last.
pub(crate) fn newest_first(mut albums: Vec<Album>) -> Vec<Album> {
    albums.sort_by_key(|album| std::cmp::Reverse(album.year()));
    albums
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn album(id: &str, artist: Option<(&str, &str)>, year: Option<u32>) -> Album {
        serde_json::from_value(json!({
            "id": id,
            "name": id,
            "artistId": artist.map(|artist| artist.0),
            "artist": artist.map(|artist| artist.1),
            "coverArt": format!("cover-{id}"),
            "year": year,
        }))
        .unwrap()
    }

    #[test]
    fn saved_albums_name_each_artist_once_with_a_count() {
        let miles = Some(("m", "Miles Davis"));
        let albums = [
            album("a1", miles, None),
            album("a2", Some(("b", "bill evans")), None),
            album("a3", miles, None),
            album("a4", None, None),
        ];
        let artists = artists_of_albums(&albums);
        let names: Vec<_> = artists.iter().map(Artist::name).collect();
        assert_eq!(names, ["bill evans", "Miles Davis"]);
        assert_eq!(artists[1].album_count(), Some(2));
        assert_eq!(artists[1].cover_art().unwrap().as_str(), "cover-a1");
    }

    #[test]
    fn an_album_naming_its_artist_with_only_spaces_lists_nobody() {
        let albums = [album("a1", Some(("blank", " ")), None)];
        assert!(artists_of_albums(&albums).is_empty());
    }

    #[test]
    fn songs_come_album_by_album_in_track_order() {
        let song = |id: &str, album: &str, track: u32| -> Song {
            serde_json::from_value(json!({
                "id": id, "title": id, "album": album, "track": track,
            }))
            .unwrap()
        };
        let songs = vec![
            song("b2", "Blue", 2),
            song("a1", "Agharta", 1),
            song("b1", "Blue", 1),
        ];
        let found = album_by_album(songs);
        let ids: Vec<_> = found.iter().map(|song| song.id().as_str()).collect();
        assert_eq!(ids, ["a1", "b1", "b2"]);
    }

    #[test]
    fn newest_albums_come_first_and_those_without_a_year_last() {
        let albums = newest_first(vec![
            album("none", None, None),
            album("old", None, Some(1959)),
            album("new", None, Some(1970)),
        ]);
        let ids: Vec<_> = albums.iter().map(|album| album.id().as_str()).collect();
        assert_eq!(ids, ["new", "old", "none"]);
    }

    #[test]
    fn the_filter_ignores_case_and_keeps_the_order() {
        let artists = [
            Artist::new(ArtistId::new("1"), "Miles Davis".into(), None, None),
            Artist::new(ArtistId::new("2"), "Bill Evans".into(), None, None),
        ];
        let index = artist_index(&artists, None);
        assert_eq!(index.search(" "), [0, 1]);
        assert_eq!(index.search("EVANS"), [1]);
        assert!(index.search("nobody").is_empty());
    }
}
