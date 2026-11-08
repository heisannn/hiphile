use std::sync::Arc;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::date::parse_time;
use crate::{AlbumId, ArtistId, CoverArtId, SongId};

/// A song (`Child` with `isDir: false`).
///
/// Serializes to the server's own JSON form, so a stored copy reads back
/// like a response. Cheap to clone: a queue can hold a whole library.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Song(Arc<Fields>);

impl Serialize for Song {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Song {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Fields::deserialize(deserializer).map(|fields| Self(Arc::new(fields)))
    }
}

#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Fields {
    id: SongId,
    title: String,
    album: Option<String>,
    album_id: Option<AlbumId>,
    artist: Option<String>,
    artist_id: Option<ArtistId>,
    display_album_artist: Option<String>,
    track: Option<u32>,
    disc_number: Option<u32>,
    cover_art: Option<CoverArtId>,
    /// Seconds.
    duration: Option<u64>,
    size: Option<u64>,
    suffix: Option<String>,
    content_type: Option<String>,
    sampling_rate: Option<u32>,
    bit_depth: Option<u32>,
    channel_count: Option<u32>,
    year: Option<u32>,
    created: Option<String>,
    played: Option<String>,
}

impl Song {
    pub fn id(&self) -> &SongId {
        &self.0.id
    }

    pub fn title(&self) -> &str {
        &self.0.title
    }

    pub fn album(&self) -> Option<&str> {
        self.0.album.as_deref()
    }

    pub fn album_id(&self) -> Option<&AlbumId> {
        self.0.album_id.as_ref()
    }

    pub fn artist(&self) -> Option<&str> {
        self.0.artist.as_deref()
    }

    pub fn artist_id(&self) -> Option<&ArtistId> {
        self.0.artist_id.as_ref()
    }

    /// OpenSubsonic servers only.
    pub fn album_artist(&self) -> Option<&str> {
        self.0.display_album_artist.as_deref()
    }

    pub fn track(&self) -> Option<u32> {
        self.0.track
    }

    pub fn disc_number(&self) -> Option<u32> {
        self.0.disc_number
    }

    pub fn cover_art(&self) -> Option<&CoverArtId> {
        self.0.cover_art.as_ref()
    }

    pub fn duration(&self) -> Option<Duration> {
        self.0.duration.map(Duration::from_secs)
    }

    /// The original file's length in bytes, which `stream` sends untranscoded.
    pub fn size(&self) -> Option<u64> {
        self.0.size
    }

    /// File extension of the original file, e.g. `flac`.
    pub fn suffix(&self) -> Option<&str> {
        self.0.suffix.as_deref()
    }

    /// MIME type of the original file, e.g. `audio/flac`.
    pub fn content_type(&self) -> Option<&str> {
        self.0.content_type.as_deref()
    }

    /// Sample rate in Hz. OpenSubsonic servers only.
    pub fn sampling_rate(&self) -> Option<u32> {
        self.0.sampling_rate
    }

    /// OpenSubsonic servers only.
    pub fn bit_depth(&self) -> Option<u32> {
        self.0.bit_depth
    }

    /// OpenSubsonic servers only.
    pub fn channel_count(&self) -> Option<u32> {
        self.0.channel_count
    }

    pub fn year(&self) -> Option<u32> {
        self.0.year
    }

    /// When it was added to the library.
    pub fn created(&self) -> Option<SystemTime> {
        self.0.created.as_deref().and_then(parse_time)
    }

    /// When the user last played it, in RFC 3339. `None` if they never have,
    /// or the server does not say.
    pub fn played(&self) -> Option<&str> {
        self.0.played.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_stored_song_reads_back_as_the_server_wrote_it() {
        let written = json!({ "id": "s", "title": "So What", "albumId": "a", "track": 1 });
        let song: Song = serde_json::from_value(written).unwrap();
        assert_eq!(song.album_id().unwrap().as_str(), "a");

        let stored = serde_json::to_value(&song).unwrap();
        assert_eq!(stored["albumId"], "a");
        assert_eq!(stored["title"], "So What");
        assert_eq!(serde_json::from_value::<Song>(stored).unwrap(), song);
    }

    #[test]
    fn the_album_artist_is_the_one_to_display() {
        let song: Song = serde_json::from_value(json!({
            "id": "s",
            "title": "t",
            "displayAlbumArtist": "One & Two",
            "albumArtists": [{ "id": "1", "name": "One" }, { "id": "2", "name": "Two" }],
        }))
        .unwrap();
        assert_eq!(song.album_artist(), Some("One & Two"));

        let song: Song = serde_json::from_value(json!({ "id": "s", "title": "t" })).unwrap();
        assert_eq!(song.album_artist(), None);
    }

    #[test]
    fn a_clone_shares_the_song() {
        let song: Song = serde_json::from_value(json!({ "id": "s", "title": "t" })).unwrap();
        assert!(Arc::ptr_eq(&song.0, &song.clone().0));
    }
}
