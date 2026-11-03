use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{AlbumId, ArtistId, CoverArtId, SongId};

/// A song (`Child` with `isDir: false`).
///
/// Serializes to the server's own JSON form, so a stored copy reads back
/// like a response.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Song {
    id: SongId,
    title: String,
    album: Option<String>,
    album_id: Option<AlbumId>,
    artist: Option<String>,
    artist_id: Option<ArtistId>,
    track: Option<u32>,
    disc_number: Option<u32>,
    cover_art: Option<CoverArtId>,
    /// Seconds.
    duration: Option<u64>,
    /// Bytes in the original file.
    size: Option<u64>,
    suffix: Option<String>,
    content_type: Option<String>,
    sampling_rate: Option<u32>,
    bit_depth: Option<u32>,
    channel_count: Option<u32>,
}

impl Song {
    pub fn id(&self) -> &SongId {
        &self.id
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn album(&self) -> Option<&str> {
        self.album.as_deref()
    }

    pub fn album_id(&self) -> Option<&AlbumId> {
        self.album_id.as_ref()
    }

    pub fn artist(&self) -> Option<&str> {
        self.artist.as_deref()
    }

    pub fn artist_id(&self) -> Option<&ArtistId> {
        self.artist_id.as_ref()
    }

    pub fn track(&self) -> Option<u32> {
        self.track
    }

    pub fn disc_number(&self) -> Option<u32> {
        self.disc_number
    }

    pub fn cover_art(&self) -> Option<&CoverArtId> {
        self.cover_art.as_ref()
    }

    pub fn duration(&self) -> Option<Duration> {
        self.duration.map(Duration::from_secs)
    }

    /// The original file's length in bytes, which `stream` sends untranscoded.
    pub fn size(&self) -> Option<u64> {
        self.size
    }

    /// File extension of the original file, e.g. `flac`.
    pub fn suffix(&self) -> Option<&str> {
        self.suffix.as_deref()
    }

    /// MIME type of the original file, e.g. `audio/flac`.
    pub fn content_type(&self) -> Option<&str> {
        self.content_type.as_deref()
    }

    /// Sample rate in Hz. OpenSubsonic servers only.
    pub fn sampling_rate(&self) -> Option<u32> {
        self.sampling_rate
    }

    /// OpenSubsonic servers only.
    pub fn bit_depth(&self) -> Option<u32> {
        self.bit_depth
    }

    /// OpenSubsonic servers only.
    pub fn channel_count(&self) -> Option<u32> {
        self.channel_count
    }
}
