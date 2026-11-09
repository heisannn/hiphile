use subsonic::{Album, AlbumId, Song, SongId};

use crate::Shelf;

/// The most album cards across the home screen. Each shelf asks the server for as many albums as
/// its rows hold at this width, so a wider window makes the cards wider rather than leave gaps.
pub const MAX_COLUMNS: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlbumShelf {
    Recommended,
    MostPlayed,
    New,
}

impl AlbumShelf {
    /// How many rows of cards the shelf shows.
    pub const fn rows(self) -> usize {
        match self {
            Self::Recommended | Self::MostPlayed => 1,
            Self::New => 2,
        }
    }

    /// How many albums fill its rows at their widest.
    pub(crate) const fn room(self) -> usize {
        self.rows() * MAX_COLUMNS
    }
}

/// By identity, so that the selection survives a reload. An album can be on more
/// than one shelf, and each of its cards is an item of its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Song(SongId),
    Album(AlbumShelf, AlbumId),
}

/// A part of the home screen, in the order they show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HomeSection {
    Recommended,
    RecentlyPlayed,
    MostPlayed,
    NewAlbums,
}

/// The sections that show. A shelf other than the new albums with nothing to show goes away, as a
/// server with no plays or albums yet has nothing to say about it; the new albums stay to say the
/// server has none. A shelf loading or failed keeps its place.
pub fn sections(
    recommended: &Shelf<Album>,
    recently_played: &Shelf<Song>,
    most_played: &Shelf<Album>,
) -> Vec<HomeSection> {
    [
        (HomeSection::Recommended, recommended.has_loaded_nothing()),
        (
            HomeSection::RecentlyPlayed,
            recently_played.has_loaded_nothing(),
        ),
        (HomeSection::MostPlayed, most_played.has_loaded_nothing()),
        (HomeSection::NewAlbums, false),
    ]
    .into_iter()
    .filter_map(|(section, is_gone)| (!is_gone).then_some(section))
    .collect()
}

#[cfg(test)]
mod tests;
