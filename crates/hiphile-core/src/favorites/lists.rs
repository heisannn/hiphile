use std::collections::HashSet;
use std::sync::Arc;

use subsonic::{Album, Artist, Song, StarTarget, Starred};

use super::Favorite;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Songs,
    Albums,
    Artists,
}

impl Section {
    /// In the order the screen shows them.
    pub const ALL: [Self; 3] = [Self::Songs, Self::Albums, Self::Artists];

    pub fn ix(self) -> usize {
        match self {
            Self::Songs => 0,
            Self::Albums => 1,
            Self::Artists => 2,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Songs => "Songs",
            Self::Albums => "Albums",
            Self::Artists => "Artists",
        }
    }

    pub fn of(target: &StarTarget) -> Self {
        match target {
            StarTarget::Song(_) => Self::Songs,
            StarTarget::Album(_) => Self::Albums,
            StarTarget::Artist(_) => Self::Artists,
        }
    }
}

/// The user's favorites of each kind, in the server's order. One added here
/// goes first, as it does on Navidrome, which lists the newest first.
#[derive(Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct FavoriteLists {
    pub songs: Arc<Vec<Song>>,
    pub albums: Arc<Vec<Album>>,
    pub artists: Arc<Vec<Artist>>,
}

impl FavoriteLists {
    pub fn new(starred: Starred) -> Self {
        let (songs, albums, artists) = starred.into_parts();
        Self {
            songs: Arc::new(songs),
            albums: Arc::new(albums),
            artists: Arc::new(artists),
        }
    }

    pub fn targets(&self) -> HashSet<StarTarget> {
        let songs = self
            .songs
            .iter()
            .map(|song| StarTarget::Song(song.id().clone()));
        let albums = self
            .albums
            .iter()
            .map(|album| StarTarget::Album(album.id().clone()));
        let artists = self
            .artists
            .iter()
            .map(|artist| StarTarget::Artist(artist.id().clone()));
        songs.chain(albums).chain(artists).collect()
    }

    pub fn len(&self, section: Section) -> usize {
        match section {
            Section::Songs => self.songs.len(),
            Section::Albums => self.albums.len(),
            Section::Artists => self.artists.len(),
        }
    }

    pub fn favorite_at(&self, section: Section, ix: usize) -> Option<Favorite> {
        match section {
            Section::Songs => self.songs.get(ix).cloned().map(Favorite::Song),
            Section::Albums => self.albums.get(ix).cloned().map(Favorite::Album),
            Section::Artists => self.artists.get(ix).cloned().map(Favorite::Artist),
        }
    }

    pub fn position(&self, target: &StarTarget) -> Option<usize> {
        match target {
            StarTarget::Song(id) => self.songs.iter().position(|song| song.id() == id),
            StarTarget::Album(id) => self.albums.iter().position(|album| album.id() == id),
            StarTarget::Artist(id) => self.artists.iter().position(|artist| artist.id() == id),
        }
    }

    pub fn contains(&self, target: &StarTarget) -> bool {
        self.position(target).is_some()
    }

    /// Puts `favorite` at `ix`, or at the end when that is past it, unless it
    /// is there already.
    pub fn add(&mut self, favorite: Favorite, ix: usize) {
        if self.contains(&favorite.target()) {
            return;
        }
        match favorite {
            Favorite::Song(song) => insert(&mut self.songs, ix, song),
            Favorite::Album(album) => insert(&mut self.albums, ix, album),
            Favorite::Artist(artist) => insert(&mut self.artists, ix, artist),
        }
    }

    pub fn remove(&mut self, target: &StarTarget) -> Option<(Favorite, usize)> {
        let ix = self.position(target)?;
        let favorite = self.favorite_at(Section::of(target), ix)?;
        match target {
            StarTarget::Song(_) => {
                Arc::make_mut(&mut self.songs).remove(ix);
            }
            StarTarget::Album(_) => {
                Arc::make_mut(&mut self.albums).remove(ix);
            }
            StarTarget::Artist(_) => {
                Arc::make_mut(&mut self.artists).remove(ix);
            }
        }
        Some((favorite, ix))
    }
}

fn insert<T: Clone>(list: &mut Arc<Vec<T>>, ix: usize, item: T) {
    let list = Arc::make_mut(list);
    list.insert(ix.min(list.len()), item);
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use subsonic::{AlbumId, ArtistId, SongId};

    use super::*;

    fn song(id: &str) -> Song {
        serde_json::from_value(json!({ "id": id, "title": id })).unwrap()
    }

    fn lists() -> FavoriteLists {
        let starred = serde_json::from_value(json!({
            "song": [{ "id": "s1", "title": "One" }, { "id": "s2", "title": "Two" }],
            "album": [{ "id": "al", "name": "Album" }],
            "artist": [{ "id": "ar", "name": "Artist" }],
        }))
        .unwrap();
        FavoriteLists::new(starred)
    }

    fn song_ids(lists: &FavoriteLists) -> Vec<&str> {
        lists.songs.iter().map(|song| song.id().as_str()).collect()
    }

    #[test]
    fn the_same_id_in_another_kind_is_another_favorite() {
        let lists = lists();
        assert!(lists.contains(&StarTarget::Album(AlbumId::new("al"))));
        assert!(!lists.contains(&StarTarget::Artist(ArtistId::new("al"))));
        assert_eq!(lists.targets().len(), 4);
    }

    #[test]
    fn what_is_added_goes_where_it_is_asked_to_once() {
        let mut lists = lists();
        lists.add(Favorite::Song(song("s3")), 0);
        lists.add(Favorite::Song(song("s3")), 2);
        assert_eq!(song_ids(&lists), ["s3", "s1", "s2"]);

        lists.add(Favorite::Song(song("s4")), 99);
        assert_eq!(song_ids(&lists), ["s3", "s1", "s2", "s4"]);
    }

    #[test]
    fn removing_says_where_it_was_so_it_can_go_back() {
        let mut lists = lists();
        let target = StarTarget::Song(SongId::new("s2"));

        let (removed, ix) = lists.remove(&target).unwrap();
        assert_eq!(removed.target(), target);
        assert_eq!(song_ids(&lists), ["s1"]);
        assert!(lists.remove(&target).is_none());

        lists.add(removed, ix);
        assert_eq!(song_ids(&lists), ["s1", "s2"]);
    }
}
