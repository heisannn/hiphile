mod lists;

use subsonic::{Album, Artist, Song, StarTarget};

pub use lists::{FavoriteLists, Section};

#[derive(Clone, Debug, PartialEq)]
pub enum Favorite {
    Song(Song),
    Album(Album),
    Artist(Artist),
}

impl Favorite {
    pub fn target(&self) -> StarTarget {
        match self {
            Self::Song(song) => StarTarget::Song(song.id().clone()),
            Self::Album(album) => StarTarget::Album(album.id().clone()),
            Self::Artist(artist) => StarTarget::Artist(artist.id().clone()),
        }
    }
}
