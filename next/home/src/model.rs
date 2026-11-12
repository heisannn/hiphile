//! What Home shows, apart from how: its sections, how many of a shelf's albums fit, what the keys
//! move through, and what a command on the selection does.

use std::sync::Arc;

use hiphile_core::Shelf;
use hiphile_core::favorites::Favorite;
use hiphile_core::home::{
    AlbumShelf, HomeSection, Item, RecommendedStatus, ShelvesStatus, sections,
};
use hiphile_core::navigation::Location;
use next_ui::card_grid::GridMotion;
use next_ui::keymap::Keystroke;
use next_ui::menus::Songs;
use next_ui::{AlbumAction, Command};
use playback::Placement;
use subsonic::{Album, Song, SongId};

/// What the shelves hold, as the core last said.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Shelves {
    pub(crate) recommended: Shelf<Album>,
    pub(crate) recent: Shelf<Song>,
    pub(crate) most_played: Shelf<Album>,
    pub(crate) new_albums: Shelf<Album>,
}

impl Shelves {
    pub(crate) fn of(recommended: &RecommendedStatus, shelves: &ShelvesStatus) -> Self {
        Self {
            recommended: recommended.albums().clone(),
            recent: shelves.recently_played().clone(),
            most_played: shelves.most_played().clone(),
            new_albums: shelves.new_albums().clone(),
        }
    }

    pub(crate) fn sections(&self) -> Vec<HomeSection> {
        sections(&self.recommended, &self.recent, &self.most_played)
    }

    pub(crate) fn albums(&self, shelf: AlbumShelf) -> &Shelf<Album> {
        match shelf {
            AlbumShelf::Recommended => &self.recommended,
            AlbumShelf::MostPlayed => &self.most_played,
            AlbumShelf::New => &self.new_albums,
        }
    }

    pub(crate) fn shown(&self, shelf: AlbumShelf, columns: usize) -> &[Album] {
        shown(self.albums(shelf).items(), shelf, columns)
    }

    /// What the keys move through, row by row from the top, with `columns` cards across.
    pub(crate) fn rows(&self, columns: usize) -> Vec<Vec<Item>> {
        let columns = columns.max(1);
        let mut rows = Vec::new();
        for section in self.sections() {
            match shelf_of(section) {
                Some(shelf) => rows.extend(self.shown(shelf, columns).chunks(columns).map(|row| {
                    row.iter()
                        .map(|album| Item::Album(shelf, album.id().clone()))
                        .collect()
                })),
                None => rows.extend(
                    self.recent
                        .items()
                        .iter()
                        .map(|song| vec![Item::Song(song.id().clone())]),
                ),
            }
        }
        rows
    }

    /// What `item` is while it shows with `columns` cards across. A narrower pane or a reload can
    /// take it off the screen, and a command must not act on what cannot be seen.
    pub(crate) fn target(&self, item: &Item, columns: usize) -> Option<Target> {
        match item {
            Item::Song(id) => {
                let Shelf::Loaded(songs) = &self.recent else {
                    return None;
                };
                let ix = song_ix(songs, id)?;
                Some(Target::Song {
                    songs: songs.clone(),
                    ix,
                })
            }
            Item::Album(shelf, id) => self
                .shown(*shelf, columns)
                .iter()
                .find(|album| album.id() == id)
                .cloned()
                .map(Target::Album),
        }
    }
}

/// Those of `albums` on `shelf` that show with `columns` cards across: as many as fill the shelf's
/// rows.
pub(crate) fn shown(albums: &[Album], shelf: AlbumShelf, columns: usize) -> &[Album] {
    &albums[..albums.len().min(columns.max(1) * shelf.rows())]
}

/// The shelf of cards a section shows, or `None` for the songs.
pub(crate) fn shelf_of(section: HomeSection) -> Option<AlbumShelf> {
    match section {
        HomeSection::Recommended => Some(AlbumShelf::Recommended),
        HomeSection::RecentlyPlayed => None,
        HomeSection::MostPlayed => Some(AlbumShelf::MostPlayed),
        HomeSection::NewAlbums => Some(AlbumShelf::New),
    }
}

pub(crate) fn section_with(shelf: AlbumShelf) -> HomeSection {
    match shelf {
        AlbumShelf::Recommended => HomeSection::Recommended,
        AlbumShelf::MostPlayed => HomeSection::MostPlayed,
        AlbumShelf::New => HomeSection::NewAlbums,
    }
}

/// The section that shows `item`.
pub(crate) fn section_of(item: &Item) -> HomeSection {
    match item {
        Item::Song(_) => HomeSection::RecentlyPlayed,
        Item::Album(shelf, _) => section_with(*shelf),
    }
}

pub(crate) fn song_ix(songs: &[Song], id: &SongId) -> Option<usize> {
    songs.iter().position(|song| song.id() == id)
}

/// What a command on the selection acts on.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Target {
    Album(Album),
    /// The one at `ix` of the songs played last, from which Play plays on.
    Song {
        songs: Arc<[Song]>,
        ix: usize,
    },
}

impl Target {
    pub(crate) fn favorite(&self) -> Favorite {
        match self {
            Self::Album(album) => Favorite::Album(album.clone()),
            Self::Song { songs, ix } => Favorite::Song(songs[*ix].clone()),
        }
    }

    /// The songs that the menus' commands act on.
    pub(crate) fn songs(&self) -> Songs {
        match self {
            Self::Album(album) => Songs::Album(album.id().clone()),
            Self::Song { songs, ix } => Songs::List(Arc::from([songs[*ix].clone()])),
        }
    }

    /// What Play does: an album plays whole, and a song plays with those after it.
    pub(crate) fn play(&self) -> Option<Command> {
        match self {
            Self::Album(_) => self.songs().command(AlbumAction::Play),
            Self::Song { songs, ix } => Some(Command::Play {
                songs: songs.clone(),
                start: Some(*ix),
            }),
        }
    }

    /// What `intent` does to it. `None` when it has nowhere to go, such as the artist of an album
    /// the server names none for.
    pub(crate) fn command(&self, intent: Intent) -> Option<Command> {
        let show = |location| Some(Command::Show(location));
        match (intent, self) {
            (Intent::Open, Self::Album(album)) | (Intent::ShowInLibrary, Self::Album(album)) => {
                show(Location::Albums(Some(album.id().clone())))
            }
            (Intent::Open | Intent::Play, _) => self.play(),
            (Intent::Enqueue(placement), _) => {
                self.songs().command(AlbumAction::Enqueue(placement))
            }
            (Intent::ShowInLibrary, Self::Song { songs, ix }) => {
                show(Location::Albums(Some(songs[*ix].album_id()?.clone())))
            }
            (Intent::GoToArtist, Self::Album(album)) => {
                show(Location::Artists(Some(album.artist_id()?.clone())))
            }
            (Intent::GoToArtist, Self::Song { songs, ix }) => {
                show(Location::Artists(Some(songs[*ix].artist_id()?.clone())))
            }
            (Intent::ToggleFavorite, _) => Some(Command::ToggleFavorite(self.favorite())),
            (Intent::ChoosePlaylist, _) => self.songs().command(AlbumAction::ChoosePlaylist),
        }
    }
}

/// What can be done to the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Intent {
    /// An album shows in Albums, and a song plays with those after it.
    Open,
    Play,
    Enqueue(Placement),
    /// An album, or a song's album, shows in Albums.
    ShowInLibrary,
    GoToArtist,
    ToggleFavorite,
    ChoosePlaylist,
}

/// What a key asks of Home.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Asked {
    Move(GridMotion),
    Do(Intent),
}

impl Asked {
    /// The keys, and what each asks for.
    pub(crate) fn strokes() -> Vec<(Keystroke, Self)> {
        let enter = || Keystroke::named(anti_webui::prelude::NamedKey::Enter);
        let intents = [
            (enter(), Intent::Open),
            (enter().with_primary(), Intent::Play),
            (enter().with_alt(), Intent::Enqueue(Placement::Next)),
            (enter().with_shift(), Intent::Enqueue(Placement::Last)),
            (Keystroke::primary('l'), Intent::ShowInLibrary),
            (Keystroke::primary('a').with_shift(), Intent::GoToArtist),
            (Keystroke::primary('d'), Intent::ToggleFavorite),
            (Keystroke::primary('p'), Intent::ChoosePlaylist),
        ];
        let moves = GridMotion::strokes().map(|(stroke, motion)| (stroke, Self::Move(motion)));
        moves
            .into_iter()
            .chain(intents.map(|(stroke, intent)| (stroke, Self::Do(intent))))
            .collect()
    }

    /// Whether a button in the page that has the focus is pressed by the key instead, as Enter
    /// clicks what has the focus.
    pub(crate) fn is_for_buttons(&self) -> bool {
        matches!(
            self,
            Self::Do(Intent::Open | Intent::Play | Intent::Enqueue(_))
        )
    }
}

#[cfg(test)]
mod tests;
