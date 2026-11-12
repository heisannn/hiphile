//! The cover at the start of a row of an album, a song or an artist.

use anti_webui::prelude::*;
use hiphile_core::Workspace;
use subsonic::CoverArtId;

use crate::cover::{CoverKind, cover};

/// A cover's side in a row of [`ROW_HEIGHT`](crate::row_list::ROW_HEIGHT).
pub const ROW_COVER: f32 = 40.0;
/// Twice the cover's side, for screens of two pixels to a point.
const PIXELS: u32 = 80;
const CORNER: f32 = 8.0;
/// More than half of the cover, which its corners are cut down to: a circle.
const ROUND: f32 = 10_000.0;

/// The cover `cover_art` names on the server of `workspace`: square with rounded corners for an
/// album or a song, a circle for an artist.
pub fn row_cover(workspace: &Workspace, cover_art: Option<&CoverArtId>, kind: CoverKind) -> Div {
    let url = cover_art.map(|cover_art| workspace.session().cover_art_url(cover_art, PIXELS));
    let corner = match kind {
        CoverKind::Album => CORNER,
        CoverKind::Artist => ROUND,
    };
    cover(url, kind).size(ROW_COVER).rounded(corner)
}
