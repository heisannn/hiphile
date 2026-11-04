//! Local copy of a server's library, shared by the GUI and the CLI.
//!
//! Each server gets one SQLite database in the system cache directory,
//! holding its albums and songs as the server sent them. [`sync_albums`] and
//! [`sync_songs`] fetch a whole collection page by page and then remove what
//! the server no longer has. Albums that were opened keep the songs fetched
//! for them, through [`Catalog::put_album`].
//!
//! [`CoverStore`] keeps cover art images, and [`SongStore`] audio files,
//! beside each server's library.
//!
//! It is a cache: a database written by another version of the schema, or
//! one that cannot be read, is emptied and filled again by the next sync.
//! Credentials are never stored.
//!
//! [`Scrobbles`] is the exception: plays the server has yet to hear of, which
//! nothing could fetch again, are kept apart from the cache.

mod blobs;
mod covers;
mod scrobbles;
mod songs;
mod store;
mod sync;

pub use covers::{COVER_CAPACITY, COVER_REFRESH_AFTER, CoverStore, KeptCover};
pub use scrobbles::{ScrobbleTarget, Scrobbles};
pub use songs::{KeptSong, SONG_CAPACITY, SongStore};
pub use store::{Catalog, Recommended};
pub use sync::{Mode, Outcome, STALE_AFTER, Source, sync_albums, sync_songs};

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the system has no cache directory, so there is nowhere to keep the library")]
    NoCacheDir,
    #[error("the system has no data directory, so there is nowhere to keep plays")]
    NoDataDir,
    #[error("could not use {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("the library cache failed: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("the library cache holds an entry it cannot read: {0}")]
    Unreadable(#[from] serde_json::Error),
    #[error(transparent)]
    Client(#[from] client::Error),
}

impl Error {
    /// The failure of the server, or of the way to it, when that is what
    /// failed.
    pub fn server_error(&self) -> Option<&client::Error> {
        match self {
            Self::Client(error) => Some(error),
            _ => None,
        }
    }

    /// Whether another connection, such as the CLI's, held the cache longer
    /// than this one waits.
    pub fn is_busy(&self) -> bool {
        store::is_busy(self)
    }

    /// Whether the cache holds something it cannot read. A sync that fetches
    /// everything again writes over unreadable entries.
    pub fn is_unreadable(&self) -> bool {
        matches!(self, Self::Unreadable(_)) || store::is_damaged(self)
    }
}
