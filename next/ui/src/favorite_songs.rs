//! Adding songs chosen together to the favorites, or taking them out, as the menu of a selection
//! and its key do.

use std::sync::Arc;

use anti_webui::prelude::untrack;
use anti_webui_md3e::components::{MenuItem, menu_item};
use hiphile_core::favorites::Favorite;
use subsonic::{Song, StarTarget};

use crate::favorite_button::favorite_label;
use crate::{Command, Commands, Known};

/// Whether every one of `songs` is a favorite. `None` for no songs, and until the favorites are
/// known. It tracks them.
pub fn are_favorites(known: Known, songs: &[Song]) -> Option<bool> {
    if songs.is_empty() {
        return None;
    }
    songs.iter().try_fold(true, |all, song| {
        let is = known.is_favorite(&StarTarget::Song(song.id().clone()))?;
        Some(all && is)
    })
}

/// Takes every one of `songs` out of the favorites when all are in, and else adds those that are
/// not. Nothing happens until the favorites are known.
pub fn toggle_favorite_songs(commands: &Commands, known: Known, songs: &[Song]) {
    let Some(all) = untrack(|| are_favorites(known, songs)) else {
        return;
    };
    for song in songs {
        let target = StarTarget::Song(song.id().clone());
        if untrack(|| known.is_favorite(&target)) == Some(all) {
            commands.send(Command::ToggleFavorite(Favorite::Song(song.clone())));
        }
    }
}

/// Add to Favorites or Remove from Favorites, by whether all of `songs` are favorites now, which
/// does [`toggle_favorite_songs`]. It is disabled until the favorites are known.
pub fn favorite_songs_item(commands: &Commands, known: Known, songs: Arc<[Song]>) -> MenuItem {
    let label = favorite_label(untrack(|| are_favorites(known, &songs)));
    let commands = commands.clone();
    let known_now = songs.clone();
    menu_item(label)
        .disabled_when(move || are_favorites(known, &known_now).is_none())
        .on_click(move || toggle_favorite_songs(&commands, known, &songs))
}

#[cfg(test)]
mod tests;
