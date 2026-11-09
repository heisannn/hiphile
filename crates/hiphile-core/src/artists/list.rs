use std::cmp::Ordering;
use std::collections::HashMap;
use std::time::SystemTime;

use fuzzy::{Index, Speller};
use subsonic::{Album, Artist, ArtistId, CoverArtId, ReleaseDate, Song};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ArtistOrder {
    /// The server's name order. Ascending under a filter, the best match comes first instead.
    #[default]
    Name,
    /// By the album added last.
    Added,
    /// By the album that came out last.
    Released,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ArtistSort {
    pub order: ArtistOrder,
    pub is_descending: bool,
}

impl ArtistSort {
    /// How `order` sorts when chosen: names from the start, dates newest first.
    pub fn chosen(order: ArtistOrder) -> Self {
        Self {
            order,
            is_descending: order != ArtistOrder::Name,
        }
    }
}

/// Each artist's newest album, by when it was added and by when it came out.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ArtistDates(HashMap<ArtistId, (Option<SystemTime>, Option<ReleaseDate>)>);

impl ArtistDates {
    pub fn of_albums(albums: &[Album]) -> Self {
        let mut dates: HashMap<ArtistId, (Option<SystemTime>, Option<ReleaseDate>)> =
            HashMap::new();
        for album in albums {
            let Some(artist) = album.artist_id() else {
                continue;
            };
            let newest = dates.entry(artist.clone()).or_default();
            newest.0 = newest.0.max(album.created());
            newest.1 = newest.1.max(album.release_date());
        }
        Self(dates)
    }

    fn added(&self, artist: &ArtistId) -> Option<SystemTime> {
        self.0.get(artist).and_then(|dates| dates.0)
    }

    fn released(&self, artist: &ArtistId) -> Option<ReleaseDate> {
        self.0.get(artist).and_then(|dates| dates.1)
    }
}

/// `found` holds positions in `artists`, which come in name order, and keeps its order among
/// artists that compare the same. Artists without a dated album come last either way.
pub fn arrange(
    artists: &[Artist],
    mut found: Vec<usize>,
    dates: &ArtistDates,
    sort: ArtistSort,
) -> Vec<usize> {
    let is_descending = sort.is_descending;
    match sort.order {
        ArtistOrder::Name if is_descending => found.sort_unstable_by(|a, b| b.cmp(a)),
        ArtistOrder::Name => {}
        ArtistOrder::Added => {
            found = by_key(found, is_descending, |ix| dates.added(artists[ix].id()));
        }
        ArtistOrder::Released => {
            found = by_key(found, is_descending, |ix| dates.released(artists[ix].id()));
        }
    }
    found
}

fn by_key<K: Ord>(
    found: Vec<usize>,
    is_descending: bool,
    key: impl Fn(usize) -> Option<K>,
) -> Vec<usize> {
    let mut keyed: Vec<(Option<K>, usize)> = found.into_iter().map(|ix| (key(ix), ix)).collect();
    keyed.sort_by(|(a, _), (b, _)| match (a, b) {
        (Some(a), Some(b)) if is_descending => b.cmp(a),
        (Some(a), Some(b)) => a.cmp(b),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    });
    keyed.into_iter().map(|(_, ix)| ix).collect()
}

pub fn artist_index(artists: &[Artist], speller: Option<&dyn Speller>) -> Index {
    Index::new(artists.iter().map(|artist| [artist.name()]), speller)
}

pub fn artists_of_albums(albums: &[Album]) -> Vec<Artist> {
    let mut found: HashMap<&ArtistId, (&str, Option<&CoverArtId>, u32)> = HashMap::new();
    for album in albums {
        let (Some(id), Some(name)) = (album.artist_id(), album.artist()) else {
            continue;
        };
        // Tags can name an artist with nothing but spaces: the server knows better
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

pub fn album_by_album(mut songs: Vec<Song>) -> Vec<Song> {
    songs.sort_by_cached_key(|song| {
        let album = song.album().unwrap_or_default().to_lowercase();
        (album, song.disc_number(), song.track())
    });
    songs
}

/// Those without a year come last.
pub fn newest_first(mut albums: Vec<Album>) -> Vec<Album> {
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
    fn artists_go_by_their_newest_album_and_those_without_one_last() {
        let dated = |id: &str, artist: &str, year: u32, created: &str| -> Album {
            serde_json::from_value(json!({
                "id": id, "name": id, "artistId": artist, "year": year, "created": created,
            }))
            .unwrap()
        };
        let albums = [
            dated("m1", "m", 1959, "2025-01-01T00:00:00Z"),
            dated("m2", "m", 1970, "2024-01-01T00:00:00Z"),
            dated("b1", "b", 1961, "2024-06-01T00:00:00Z"),
        ];
        let dates = ArtistDates::of_albums(&albums);
        let artists = [
            Artist::new(ArtistId::new("b"), "Bill Evans".into(), None, None),
            Artist::new(ArtistId::new("n"), "Nobody".into(), None, None),
            Artist::new(ArtistId::new("m"), "Miles Davis".into(), None, None),
        ];
        let ids = |order| -> Vec<&str> {
            arrange(&artists, vec![0, 1, 2], &dates, ArtistSort::chosen(order))
                .into_iter()
                .map(|ix| artists[ix].id().as_str())
                .collect()
        };
        assert_eq!(ids(ArtistOrder::Name), ["b", "n", "m"]);
        assert_eq!(ids(ArtistOrder::Added), ["m", "b", "n"]);
        assert_eq!(ids(ArtistOrder::Released), ["m", "b", "n"]);
        let older_release = [
            dated("m1", "m", 1950, "2025-01-01T00:00:00Z"),
            albums[2].clone(),
        ];
        let dates = ArtistDates::of_albums(&older_release);
        let released = ArtistSort::chosen(ArtistOrder::Released);
        assert_eq!(
            arrange(&artists, vec![0, 1, 2], &dates, released),
            [0, 2, 1]
        );
        let oldest_first = ArtistSort {
            is_descending: false,
            ..released
        };
        assert_eq!(
            arrange(&artists, vec![0, 1, 2], &dates, oldest_first),
            [2, 0, 1]
        );
        let names_from_the_end = ArtistSort {
            order: ArtistOrder::Name,
            is_descending: true,
        };
        assert_eq!(
            arrange(&artists, vec![0, 2], &dates, names_from_the_end),
            [2, 0]
        );
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
