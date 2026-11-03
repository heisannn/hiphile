//! Actions that one feature dispatches and another handles.
//!
//! Features never depend on each other: `library` dispatches [`PlaySongs`]
//! and the app shell routes it to `player`. Only cross-feature actions belong
//! here; a feature's own commands stay in that feature.

use std::sync::Arc;

use gpui_kit::Action;
use subsonic::Song;

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
