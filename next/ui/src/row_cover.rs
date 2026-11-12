//! The cover at the start of a row of an album, a song or an artist.

use anti_webui::prelude::*;
use hiphile_core::Workspace;
use subsonic::CoverArtId;

use crate::cover::{CoverKind, ROUND, cover};

/// A cover's side in a row of [`ROW_HEIGHT`](crate::row_list::ROW_HEIGHT).
pub const ROW_COVER: f32 = 40.0;
/// What `next_player` asks the bar's cover at, so that the song that plays needs no second image.
const PIXELS: u32 = 128;
const CORNER: f32 = 8.0;

/// The cover `cover_art` names on the server of `workspace`: square with rounded corners for an
/// album or a song, a circle for an artist.
pub fn row_cover(workspace: &Workspace, cover_art: Option<&CoverArtId>, kind: CoverKind) -> Div {
    let url = cover_art.map(|cover_art| workspace.session().cover_art_url(cover_art, PIXELS));
    let corner = match kind {
        CoverKind::Album | CoverKind::Playlist => CORNER,
        CoverKind::Artist => ROUND,
    };
    cover(url, kind).size(ROW_COVER).rounded(corner)
}

#[cfg(test)]
mod tests;
