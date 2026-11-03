//! Actions that one feature dispatches and another handles.
//!
//! Features never depend on each other: `library` dispatches [`PlaySongs`]
//! and the app shell routes it to `player`. Only cross-feature actions belong
//! here; a feature's own commands stay in that feature.

use std::sync::Arc;

use gpui_kit::Action;
use subsonic::{AlbumId, Song};

/// Show `album` in the library.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = hiphile, no_json)]
pub struct ShowAlbum {
    album: AlbumId,
}

impl ShowAlbum {
    pub fn new(album: AlbumId) -> Self {
        Self { album }
    }

    pub fn album(&self) -> &AlbumId {
        &self.album
    }
}

/// Replace the play queue with `songs` and start playing at `start_ix`.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = hiphile, no_json)]
pub struct PlaySongs {
    songs: Arc<[Song]>,
    start_ix: usize,
}

impl PlaySongs {
    pub fn new(songs: Arc<[Song]>, start_ix: usize) -> Self {
        Self { songs, start_ix }
    }

    pub fn songs(&self) -> &Arc<[Song]> {
        &self.songs
    }

    pub fn start_ix(&self) -> usize {
        self.start_ix
    }
}

/// Where [`EnqueueSongs`] adds its songs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// Right after the current song.
    Next,
    /// After every other song.
    Last,
}

/// Add `songs` to the play queue at `placement`, starting them when nothing
/// is playing.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = hiphile, no_json)]
pub struct EnqueueSongs {
    songs: Arc<[Song]>,
    placement: Placement,
}

impl EnqueueSongs {
    pub fn new(songs: Arc<[Song]>, placement: Placement) -> Self {
        Self { songs, placement }
    }

    pub fn songs(&self) -> &Arc<[Song]> {
        &self.songs
    }

    pub fn placement(&self) -> Placement {
        self.placement
    }
}
