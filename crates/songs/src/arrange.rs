use std::cmp::Ordering;

use fuzzy::{Index, Speller};
use subsonic::Song;

/// A column of the table of songs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Column {
    Title,
    Artist,
    Album,
    Time,
}

/// Which column the songs follow, and which way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Sort {
    pub(crate) column: Column,
    pub(crate) is_descending: bool,
}

impl Sort {
    /// Sorted by title, from A.
    pub(crate) const DEFAULT: Self = Self {
        column: Column::Title,
        is_descending: false,
    };

    /// What clicking the heading of `column` asks for: the other way round for
    /// the column in use, otherwise that column from its start.
    pub(crate) fn clicked(self, column: Column) -> Self {
        Self {
            column,
            is_descending: self.column == column && !self.is_descending,
        }
    }
}

/// What a song is compared by, worked out once per song: lowercasing on every
/// comparison would take most of the time of sorting a large library.
struct Key {
    title: String,
    artist: Option<String>,
    album: Option<String>,
    disc: Option<u32>,
    track: Option<u32>,
    seconds: Option<u64>,
}

impl Key {
    fn new(song: &Song) -> Self {
        // Tags can start with stray spaces, which should not put them first.
        let key = |text: &str| text.trim().to_lowercase();
        Self {
            title: key(song.title()),
            artist: song.artist().map(key),
            album: song.album().map(key),
            disc: song.disc_number(),
            track: song.track(),
            seconds: song.duration().map(|duration| duration.as_secs()),
        }
    }
}

/// Compares what a column holds. A song that does not say comes last, whichever
/// way the column runs: blanks are of no interest at either end.
fn compare_column<T: Ord>(a: Option<&T>, b: Option<&T>, is_descending: bool) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) if is_descending => b.cmp(a),
        (Some(a), Some(b)) => a.cmp(b),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// Keeps an album's songs in their order.
fn compare_within_album(a: &Key, b: &Key) -> Ordering {
    compare_column(a.disc.as_ref(), b.disc.as_ref(), false)
        .then_with(|| compare_column(a.track.as_ref(), b.track.as_ref(), false))
}

fn compare(a: &Key, b: &Key, sort: Sort) -> Ordering {
    let is_descending = sort.is_descending;
    let by_title = |a: &Key, b: &Key| a.title.cmp(&b.title);
    let by_album = |a: &Key, b: &Key| {
        compare_column(a.album.as_ref(), b.album.as_ref(), false)
            .then_with(|| compare_within_album(a, b))
    };
    match sort.column {
        Column::Title => compare_column(Some(&a.title), Some(&b.title), is_descending)
            .then_with(|| compare_column(a.artist.as_ref(), b.artist.as_ref(), false))
            .then_with(|| by_album(a, b)),
        Column::Artist => compare_column(a.artist.as_ref(), b.artist.as_ref(), is_descending)
            .then_with(|| by_album(a, b))
            .then_with(|| by_title(a, b)),
        Column::Album => compare_column(a.album.as_ref(), b.album.as_ref(), is_descending)
            .then_with(|| compare_within_album(a, b))
            .then_with(|| by_title(a, b)),
        Column::Time => compare_column(a.seconds.as_ref(), b.seconds.as_ref(), is_descending)
            .then_with(|| by_title(a, b)),
    }
}

/// `songs`, to find by their title, artist or album.
pub(crate) fn song_index(songs: &[Song], speller: Option<&dyn Speller>) -> Index {
    Index::new(
        songs.iter().map(|song| {
            let title = Some(song.title());
            [title, song.artist(), song.album()].into_iter().flatten()
        }),
        speller,
    )
}

/// Positions of the songs that match `query`: the best first when `by_relevance`
/// and there is a query to rank by, otherwise in the order of `sort`. Songs that
/// compare the same go by ID, so that the order holds whatever order the songs
/// arrive in.
pub(crate) fn arrange(
    songs: &[Song],
    index: &Index,
    query: &str,
    sort: Sort,
    by_relevance: bool,
) -> Vec<usize> {
    let found = index.search(query);
    if by_relevance && !query.trim().is_empty() {
        return found;
    }
    let mut keyed: Vec<(usize, Key)> = found
        .into_iter()
        .map(|ix| (ix, Key::new(&songs[ix])))
        .collect();
    keyed.sort_unstable_by(|(a_ix, a), (b_ix, b)| {
        compare(a, b, sort).then_with(|| songs[*a_ix].id().cmp(songs[*b_ix].id()))
    });
    keyed.into_iter().map(|(ix, _)| ix).collect()
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn song(fields: Value) -> Song {
        serde_json::from_value(fields).unwrap()
    }

    /// Four songs across two albums, one of them saying little.
    fn songs() -> Vec<Song> {
        vec![
            song(json!({
                "id": "a2", "title": "blue in green", "artist": "Miles Davis",
                "album": "Kind of Blue", "track": 3, "duration": 337,
            })),
            song(json!({
                "id": "a1", "title": "So What", "artist": "Miles Davis",
                "album": "Kind of Blue", "track": 1, "duration": 562,
            })),
            song(json!({
                "id": "b1", "title": "Naima", "artist": "John Coltrane",
                "album": "Giant Steps", "track": 6, "duration": 261,
            })),
            song(json!({ "id": "c1", "title": "Untitled" })),
        ]
    }

    fn ids(songs: &[Song], query: &str, column: Column, is_descending: bool) -> Vec<String> {
        let sort = Sort {
            column,
            is_descending,
        };
        let index = song_index(songs, None);
        arrange(songs, &index, query, sort, false)
            .into_iter()
            .map(|ix| songs[ix].id().to_string())
            .collect()
    }

    #[test]
    fn titles_sort_ignoring_case_both_ways() {
        let songs = songs();
        assert_eq!(
            ids(&songs, "", Column::Title, false),
            ["a2", "b1", "a1", "c1"]
        );
        assert_eq!(
            ids(&songs, "", Column::Title, true),
            ["c1", "a1", "b1", "a2"]
        );
    }

    #[test]
    fn spaces_before_a_name_do_not_sort_it_first() {
        let songs = [
            song(json!({ "id": "a", "title": "Apple" })),
            song(json!({ "id": "b", "title": "  Banana" })),
        ];
        assert_eq!(ids(&songs, "", Column::Title, false), ["a", "b"]);
    }

    #[test]
    fn artists_keep_their_albums_in_track_order_either_way() {
        let songs = songs();
        assert_eq!(
            ids(&songs, "", Column::Artist, false),
            ["b1", "a1", "a2", "c1"]
        );
        assert_eq!(
            ids(&songs, "", Column::Artist, true),
            ["a1", "a2", "b1", "c1"]
        );
    }

    #[test]
    fn albums_keep_their_tracks_in_order_either_way() {
        let songs = songs();
        assert_eq!(
            ids(&songs, "", Column::Album, false),
            ["b1", "a1", "a2", "c1"]
        );
        assert_eq!(
            ids(&songs, "", Column::Album, true),
            ["a1", "a2", "b1", "c1"]
        );
    }

    #[test]
    fn times_sort_with_the_unknown_last_both_ways() {
        let songs = songs();
        assert_eq!(
            ids(&songs, "", Column::Time, false),
            ["b1", "a2", "a1", "c1"]
        );
        assert_eq!(
            ids(&songs, "", Column::Time, true),
            ["a1", "a2", "b1", "c1"]
        );
    }

    #[test]
    fn songs_alike_keep_one_order_however_they_arrive() {
        let alike = |id: &str| song(json!({ "id": id, "title": "Same" }));
        let forwards = [alike("x"), alike("y")];
        let backwards = [alike("y"), alike("x")];
        assert_eq!(ids(&forwards, "", Column::Title, false), ["x", "y"]);
        assert_eq!(ids(&backwards, "", Column::Title, false), ["x", "y"]);
    }

    #[test]
    fn the_filter_looks_at_title_artist_and_album_ignoring_case() {
        let songs = songs();
        assert_eq!(ids(&songs, " NAIMA ", Column::Title, false), ["b1"]);
        assert_eq!(ids(&songs, "coltrane", Column::Title, false), ["b1"]);
        assert_eq!(ids(&songs, "kind of", Column::Title, false), ["a2", "a1"]);
        assert!(ids(&songs, "nobody", Column::Title, false).is_empty());
    }

    #[test]
    fn a_query_goes_by_relevance_until_a_heading_is_asked_for() {
        let songs = [
            song(json!({ "id": "a", "title": "Kind of Blue" })),
            song(json!({ "id": "b", "title": "Blue Train" })),
        ];
        let index = song_index(&songs, None);
        let sort = Sort::DEFAULT;
        assert_eq!(arrange(&songs, &index, "blue", sort, true), [1, 0]);
        // Sorted by title, where B comes before K.
        assert_eq!(arrange(&songs, &index, "blue", sort, false), [1, 0]);
        let by_time = Sort {
            column: Column::Title,
            is_descending: true,
        };
        assert_eq!(arrange(&songs, &index, "blue", by_time, false), [0, 1]);
        // Nothing to rank by, so the sort it is.
        assert_eq!(arrange(&songs, &index, " ", by_time, true), [0, 1]);
    }

    #[test]
    fn a_heading_turns_its_column_around_and_another_starts_from_the_top() {
        let once = Sort::DEFAULT.clicked(Column::Title);
        assert_eq!(once.column, Column::Title);
        assert!(once.is_descending);
        assert!(!once.clicked(Column::Title).is_descending);
        let other = once.clicked(Column::Time);
        assert_eq!(other.column, Column::Time);
        assert!(!other.is_descending);
    }
}
