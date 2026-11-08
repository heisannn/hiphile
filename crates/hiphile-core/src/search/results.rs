use std::collections::BTreeMap;

use fuzzy::{Index, Speller};
use subsonic::{Album, ArtistId, Song};

const MAX_ARTISTS: usize = 5;
const MAX_ALBUMS: usize = 8;
const MAX_SONGS: usize = 30;

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct NamedArtist {
    pub id: ArtistId,
    pub name: String,
}

#[derive(Clone, Debug)]
pub enum Hit {
    Artist(NamedArtist),
    Album(Album),
    Song(Song),
}

/// What a search found, kind by kind, the best of each first. A kind with
/// nothing to show is left out, so positions here are positions in the palette.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Results {
    pub sections: Vec<(&'static str, Vec<Hit>)>,
}

impl Results {
    pub fn hit(&self, section: usize, row: usize) -> Option<&Hit> {
        self.sections.get(section)?.1.get(row)
    }
}

/// Takes a while to read with a dictionary: build it off the UI thread.
pub struct SearchLibrary {
    artists: Vec<NamedArtist>,
    albums: Vec<Album>,
    songs: Vec<Song>,
    artist_index: Index,
    album_index: Index,
    song_index: Index,
}

impl SearchLibrary {
    pub fn new(albums: Vec<Album>, songs: Vec<Song>, speller: Option<&dyn Speller>) -> Self {
        let artists = named_artists(&albums, &songs);
        let artist_index = Index::new(artists.iter().map(|artist| [artist.name.as_str()]), speller);
        let album_index = Index::new(
            albums.iter().map(|album| {
                let name = Some(album.name());
                [name, album.artist()].into_iter().flatten()
            }),
            speller,
        );
        let song_index = Index::new(
            songs.iter().map(|song| {
                let title = Some(song.title());
                [title, song.artist(), song.album()].into_iter().flatten()
            }),
            speller,
        );
        Self {
            artists,
            albums,
            songs,
            artist_index,
            album_index,
            song_index,
        }
    }

    pub fn search(&self, query: &str) -> Results {
        if query.trim().is_empty() {
            return Results::default();
        }
        // A mistake forgiven in one kind must not stand above what another kind
        // found as spelled: the first row is what Enter takes.
        let mut found = self.found(query, Index::search_as_spelled);
        if found.iter().all(Vec::is_empty) {
            found = self.found(query, Index::search);
        }
        let [artists, albums, songs] = found;
        let artists = artists
            .into_iter()
            .take(MAX_ARTISTS)
            .map(|ix| Hit::Artist(self.artists[ix].clone()))
            .collect();
        let albums = albums
            .into_iter()
            .take(MAX_ALBUMS)
            .map(|ix| Hit::Album(self.albums[ix].clone()))
            .collect();
        let songs = songs
            .into_iter()
            .take(MAX_SONGS)
            .map(|ix| Hit::Song(self.songs[ix].clone()))
            .collect();
        let sections = [("Artists", artists), ("Albums", albums), ("Songs", songs)]
            .into_iter()
            .filter(|(_, hits): &(_, Vec<Hit>)| !hits.is_empty())
            .collect();
        Results { sections }
    }

    fn found(&self, query: &str, search: impl Fn(&Index, &str) -> Vec<usize>) -> [Vec<usize>; 3] {
        [&self.artist_index, &self.album_index, &self.song_index].map(|it| search(it, query))
    }
}

/// Each artist that `albums` and `songs` name, once and in order of ID, leaving
/// out any named with nothing but spaces: tags can do that.
fn named_artists(albums: &[Album], songs: &[Song]) -> Vec<NamedArtist> {
    let of_albums = albums.iter().map(|it| (it.artist_id(), it.artist()));
    let of_songs = songs.iter().map(|it| (it.artist_id(), it.artist()));
    let mut found: BTreeMap<&ArtistId, &str> = BTreeMap::new();
    for pair in of_albums.chain(of_songs) {
        if let (Some(id), Some(name)) = pair
            && !name.trim().is_empty()
        {
            found.entry(id).or_insert(name);
        }
    }
    found
        .into_iter()
        .map(|(id, name)| NamedArtist {
            id: id.clone(),
            name: name.to_owned(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use fuzzy::Analyzer;

    use super::*;

    struct Reads;

    impl Analyzer for Reads {
        fn readings(&self, text: &str) -> Vec<String> {
            match text {
                "米津玄師" => vec!["ヨネヅケンシ".to_owned()],
                "迷える羊" => vec!["マヨエル ヒツジ".to_owned()],
                _ => Vec::new(),
            }
        }
    }

    fn library() -> SearchLibrary {
        let albums = serde_json::from_value(json!([
            { "id": "stray", "name": "STRAY SHEEP", "artist": "米津玄師", "artistId": "yonezu" },
            { "id": "blue", "name": "Kind of Blue", "artist": "Miles Davis", "artistId": "miles" },
        ]))
        .unwrap();
        let songs = serde_json::from_value(json!([
            { "id": "lemon", "title": "Lemon", "artist": "米津玄師", "artistId": "yonezu",
              "album": "STRAY SHEEP" },
            { "id": "sheep", "title": "迷える羊", "artist": "米津玄師", "artistId": "yonezu",
              "album": "STRAY SHEEP" },
            { "id": "alone", "title": "Alone", "artist": "Nobody Else", "artistId": "else" },
            { "id": "blank", "title": "Blank", "artist": " ", "artistId": "blank" },
        ]))
        .unwrap();
        SearchLibrary::new(albums, songs, Some(&Reads))
    }

    fn names(results: &Results) -> Vec<(&'static str, Vec<String>)> {
        results
            .sections
            .iter()
            .map(|(label, hits)| {
                let names = hits
                    .iter()
                    .map(|hit| match hit {
                        Hit::Artist(artist) => artist.name.clone(),
                        Hit::Album(album) => album.name().to_owned(),
                        Hit::Song(song) => song.title().to_owned(),
                    })
                    .collect();
                (*label, names)
            })
            .collect()
    }

    #[test]
    fn nothing_is_asked_nothing_is_found() {
        assert!(library().search("  ").sections.is_empty());
    }

    #[test]
    fn a_name_in_kanji_is_found_by_romaji_across_every_kind() {
        assert_eq!(
            names(&library().search("yonezu")),
            [
                ("Artists", vec!["米津玄師".to_owned()]),
                ("Albums", vec!["STRAY SHEEP".to_owned()]),
                ("Songs", vec!["Lemon".to_owned(), "迷える羊".to_owned()]),
            ]
        );
    }

    #[test]
    fn a_kind_with_nothing_found_is_left_out_of_the_positions() {
        let library = library();
        let results = library.search("mayoeru");
        assert_eq!(results.sections.len(), 1);
        assert!(matches!(results.hit(0, 0), Some(Hit::Song(_))));
        assert!(results.hit(1, 0).is_none());
        assert!(results.hit(0, 1).is_none());
    }

    fn lemon_and_ceremony() -> SearchLibrary {
        let albums = serde_json::from_value(json!([{ "id": "c", "name": "CEREMONY" }])).unwrap();
        let songs = serde_json::from_value(json!([{ "id": "l", "title": "Lemon" }])).unwrap();
        SearchLibrary::new(albums, songs, None)
    }

    #[test]
    fn a_kind_spelled_right_keeps_a_mistake_in_another_from_coming_first() {
        assert_eq!(
            names(&lemon_and_ceremony().search("lemon")),
            [("Songs", vec!["Lemon".to_owned()])]
        );
    }

    #[test]
    fn a_mistake_is_forgiven_when_nothing_is_spelled_that_way() {
        assert_eq!(
            names(&lemon_and_ceremony().search("lemmon")),
            [("Songs", vec!["Lemon".to_owned()])]
        );
    }

    #[test]
    fn artists_come_from_songs_as_well_as_albums_and_blank_ones_do_not() {
        let library = library();
        let found: Vec<_> = library.artists.iter().map(|it| it.name.as_str()).collect();
        assert_eq!(found, ["Nobody Else", "Miles Davis", "米津玄師"]);
    }

    #[test]
    fn each_kind_shows_only_so_many() {
        let songs = (0..MAX_SONGS + 5)
            .map(|ix| json!({ "id": format!("s{ix}"), "title": "Same" }))
            .collect();
        let songs = serde_json::from_value(serde_json::Value::Array(songs)).unwrap();
        let library = SearchLibrary::new(Vec::new(), songs, None);
        assert_eq!(library.search("same").sections[0].1.len(), MAX_SONGS);
    }
}
