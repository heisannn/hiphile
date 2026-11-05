use serde::{Deserialize, Serialize};

use crate::{Album, ArtistId, CoverArtId};

/// An artist (`ArtistID3`).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Artist {
    id: ArtistId,
    name: String,
    cover_art: Option<CoverArtId>,
    album_count: Option<u32>,
}

impl Artist {
    /// An artist known only by what an album or a song says of them.
    pub fn new(
        id: ArtistId,
        name: String,
        cover_art: Option<CoverArtId>,
        album_count: Option<u32>,
    ) -> Self {
        Self {
            id,
            name,
            cover_art,
            album_count,
        }
    }

    pub fn id(&self) -> &ArtistId {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn cover_art(&self) -> Option<&CoverArtId> {
        self.cover_art.as_ref()
    }

    pub fn album_count(&self) -> Option<u32> {
        self.album_count
    }
}

/// An artist with their albums (`ArtistWithAlbumsID3`), in server order.
#[derive(Clone, Debug, Deserialize)]
pub struct ArtistWithAlbums {
    #[serde(flatten)]
    artist: Artist,
    #[serde(default, rename = "album")]
    albums: Vec<Album>,
}

impl ArtistWithAlbums {
    pub fn into_parts(self) -> (Artist, Vec<Album>) {
        (self.artist, self.albums)
    }
}
