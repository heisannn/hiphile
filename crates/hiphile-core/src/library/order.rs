use std::cmp::Ordering;

use subsonic::Album;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AlbumOrder {
    /// The server's name order. Ascending under a filter, the best match comes first instead.
    #[default]
    Name,
    /// Each artist's albums in name order, whichever way the artists run.
    Artist,
    Added,
    Released,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct AlbumSort {
    pub order: AlbumOrder,
    pub is_descending: bool,
}

impl AlbumSort {
    /// How `order` sorts when chosen: names from the start, dates newest first.
    pub fn chosen(order: AlbumOrder) -> Self {
        Self {
            order,
            is_descending: matches!(order, AlbumOrder::Added | AlbumOrder::Released),
        }
    }
}

/// `found` holds positions in `albums`, which come in the server's name order, and keeps its
/// order among albums that compare the same. Albums that do not say come last either way.
pub fn arrange(albums: &[Album], mut found: Vec<usize>, sort: AlbumSort) -> Vec<usize> {
    match sort.order {
        AlbumOrder::Name if sort.is_descending => found.sort_unstable_by(|a, b| b.cmp(a)),
        AlbumOrder::Name => {}
        AlbumOrder::Artist => {
            found.sort_unstable();
            found = by_key(found, sort.is_descending, |ix| artist_key(&albums[ix]));
        }
        AlbumOrder::Added => {
            found = by_key(found, sort.is_descending, |ix| albums[ix].created());
        }
        AlbumOrder::Released => {
            found = by_key(found, sort.is_descending, |ix| albums[ix].release_date());
        }
    }
    found
}

/// Tags can start with stray spaces or be nothing but, which should not put them first.
fn artist_key(album: &Album) -> Option<String> {
    let artist = album.artist()?.trim();
    (!artist.is_empty()).then(|| artist.to_lowercase())
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn albums() -> Vec<Album> {
        serde_json::from_value(json!([
            { "id": "a", "name": "A", "year": 1959, "created": "2024-03-01T00:00:00Z" },
            { "id": "b", "name": "B" },
            { "id": "c", "name": "C", "year": 1970, "created": "2024-01-01T00:00:00Z" },
            { "id": "d", "name": "D", "year": 1959, "created": "2025-01-01T00:00:00Z",
              "releaseDate": { "year": 1959, "month": 8 } },
        ]))
        .unwrap()
    }

    fn ids(albums: &[Album], found: Vec<usize>, order: AlbumOrder) -> Vec<&str> {
        arrange(albums, found, AlbumSort::chosen(order))
            .into_iter()
            .map(|ix| albums[ix].id().as_str())
            .collect()
    }

    #[test]
    fn by_name_the_order_found_stays() {
        let albums = albums();
        assert_eq!(
            ids(&albums, vec![2, 0, 3, 1], AlbumOrder::Name),
            ["c", "a", "d", "b"]
        );
    }

    #[test]
    fn the_newest_come_first_and_those_that_do_not_say_last() {
        let albums = albums();
        let all = || (0..albums.len()).collect();
        assert_eq!(ids(&albums, all(), AlbumOrder::Added), ["d", "a", "c", "b"]);
        assert_eq!(
            ids(&albums, all(), AlbumOrder::Released),
            ["c", "d", "a", "b"]
        );
    }

    #[test]
    fn turned_around_the_oldest_come_first_and_those_that_do_not_say_still_last() {
        let albums = albums();
        let ids = |order| -> Vec<&str> {
            let sort = AlbumSort {
                order,
                is_descending: false,
            };
            let found = (0..albums.len()).collect();
            arrange(&albums, found, sort)
                .into_iter()
                .map(|ix| albums[ix].id().as_str())
                .collect()
        };
        assert_eq!(ids(AlbumOrder::Added), ["c", "a", "d", "b"]);
        assert_eq!(ids(AlbumOrder::Released), ["a", "d", "c", "b"]);
    }

    #[test]
    fn names_turned_around_go_from_the_end_also_under_a_filter() {
        let albums = albums();
        let sort = AlbumSort {
            order: AlbumOrder::Name,
            is_descending: true,
        };
        let shown: Vec<_> = arrange(&albums, vec![2, 0, 3], sort);
        assert_eq!(shown, [3, 2, 0]);
    }

    #[test]
    fn artists_keep_their_albums_in_name_order_either_way() {
        let albums: Vec<Album> = serde_json::from_value(json!([
            { "id": "m1", "name": "Bitches Brew", "artist": "Miles Davis" },
            { "id": "b1", "name": "Explorations", "artist": " bill evans" },
            { "id": "n", "name": "Field Recordings", "artist": " " },
            { "id": "m2", "name": "Kind of Blue", "artist": "Miles Davis" },
            { "id": "b2", "name": "Sunday at the Village Vanguard", "artist": "Bill Evans" },
        ]))
        .unwrap();
        let ids = |is_descending| -> Vec<&str> {
            let sort = AlbumSort {
                order: AlbumOrder::Artist,
                is_descending,
            };
            // A filter puts the best match first, which counts for nothing here
            arrange(&albums, vec![4, 3, 2, 1, 0], sort)
                .into_iter()
                .map(|ix| albums[ix].id().as_str())
                .collect()
        };
        assert_eq!(ids(false), ["b1", "b2", "m1", "m2", "n"]);
        assert_eq!(ids(true), ["m1", "m2", "b1", "b2", "n"]);
        assert!(!AlbumSort::chosen(AlbumOrder::Artist).is_descending);
    }

    #[test]
    fn albums_alike_keep_the_order_found() {
        let albums: Vec<Album> = serde_json::from_value(json!([
            { "id": "x", "name": "X", "year": 2000 },
            { "id": "y", "name": "Y", "year": 2000 },
        ]))
        .unwrap();
        assert_eq!(ids(&albums, vec![1, 0], AlbumOrder::Released), ["y", "x"]);
        assert_eq!(ids(&albums, vec![0, 1], AlbumOrder::Released), ["x", "y"]);
    }
}
