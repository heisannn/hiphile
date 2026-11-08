use subsonic::{AlbumId, SongId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlbumShelf {
    Recommended,
    MostPlayed,
    New,
}

/// By identity, so that the selection survives a reload. An album can be on more
/// than one shelf, and each of its cards is an item of its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Song(SongId),
    Album(AlbumShelf, AlbumId),
}
