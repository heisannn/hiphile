use std::collections::HashSet;
use std::sync::Arc;

use anti_webui::prelude::{Memo, memo};
use hiphile_core::playlists::PlaylistChoice;
use hiphile_core::settings::QueueSnapshot;
use hiphile_core::{Hiphile, Workspace};
use next_host::follow;
use subsonic::{Song, StarTarget};

/// What the commands on songs, albums and artists go by on every screen: which are favorites, the
/// playlists songs can be added to, and whether instrumentals are skipped. The shell follows it
/// once and hands it to the screens. Cheap to copy, and reading it tracks it.
#[derive(Clone, Copy)]
pub struct Known {
    favorites: Memo<Option<Arc<HashSet<StarTarget>>>>,
    playlists: Memo<Arc<[PlaylistChoice]>>,
    skips_instrumentals: Memo<bool>,
}

impl Known {
    /// Follows `workspace` and the settings of `hiphile` for as long as the current owner lives.
    pub fn follow(hiphile: &Hiphile, workspace: &Workspace) -> Self {
        let favorites = follow(workspace.favorites().subscribe());
        let playlists = follow(workspace.playlists().subscribe());
        let queue = follow(hiphile.settings().queue().subscribe());
        Self {
            favorites: memo(move || favorites.with(|favorites| favorites.known().cloned())),
            playlists: memo(move || playlists.with(|playlists| playlists.choices().clone())),
            skips_instrumentals: memo(move || queue.with(QueueSnapshot::is_skip_instrumentals_on)),
        }
    }

    /// `None` until the favorites are known.
    pub fn is_favorite(&self, target: &StarTarget) -> Option<bool> {
        self.favorites
            .with(|favorites| Some(favorites.as_ref()?.contains(target)))
    }

    /// The user's own playlists, in the order they are listed.
    pub fn playlist_choices(&self) -> Arc<[PlaylistChoice]> {
        self.playlists.get()
    }

    pub fn skips_instrumentals(&self) -> bool {
        self.skips_instrumentals.get()
    }

    /// Whether playing `song` would pass it over.
    pub fn is_skipped(&self, song: &Song) -> bool {
        playback::is_skipped(song, self.skips_instrumentals())
    }
}

#[cfg(test)]
mod tests;
