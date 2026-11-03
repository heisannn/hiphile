use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{AlbumId, ArtistId, CoverArtId, Song};

/// An album (`AlbumID3`).
///
/// Serializes to the server's own JSON form, so a stored copy reads back
/// like a response.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Album {
    id: AlbumId,
    name: String,
    artist: Option<String>,
    artist_id: Option<ArtistId>,
    cover_art: Option<CoverArtId>,
    song_count: Option<u32>,
    /// Seconds.
    duration: Option<u64>,
    year: Option<u32>,
}

impl Album {
    pub fn id(&self) -> &AlbumId {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn artist(&self) -> Option<&str> {
        self.artist.as_deref()
    }

    pub fn artist_id(&self) -> Option<&ArtistId> {
        self.artist_id.as_ref()
    }

    pub fn cover_art(&self) -> Option<&CoverArtId> {
        self.cover_art.as_ref()
    }

    pub fn song_count(&self) -> Option<u32> {
        self.song_count
    }

    pub fn duration(&self) -> Option<Duration> {
        self.duration.map(Duration::from_secs)
    }

    pub fn year(&self) -> Option<u32> {
        self.year
    }
}

/// An album with its songs (`AlbumID3WithSongs`), in server order.
#[derive(Clone, Debug, Deserialize)]
pub struct AlbumWithSongs {
    #[serde(flatten)]
    album: Album,
    #[serde(default, rename = "song")]
    songs: Vec<Song>,
}

impl AlbumWithSongs {
    pub fn new(album: Album, songs: Vec<Song>) -> Self {
        Self { album, songs }
    }

    pub fn album(&self) -> &Album {
        &self.album
    }

    pub fn songs(&self) -> &[Song] {
        &self.songs
    }

    pub fn into_parts(self) -> (Album, Vec<Song>) {
        (self.album, self.songs)
    }
}
