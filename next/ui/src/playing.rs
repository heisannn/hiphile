//! The song the player is on, which the rows of songs mark.

use anti_webui::prelude::{Memo, memo};
use hiphile_core::Workspace;
use next_host::follow;

use crate::now_playing::NowPlaying;

/// Follows the player of `workspace` for as long as the current owner lives.
pub fn follow_now_playing(workspace: &Workspace) -> Memo<Option<NowPlaying>> {
    let player = follow(workspace.player().subscribe());
    memo(move || {
        player.with(|player| {
            let song = player.current_song().map(|song| song.id().clone());
            NowPlaying::new(song, player.is_playing())
        })
    })
}
