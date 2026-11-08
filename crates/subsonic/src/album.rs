use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::date::parse_time;
use crate::{AlbumId, ArtistId, CoverArtId, ReleaseDate, Song};

/// An album (`AlbumID3`).
///
/// Serializes to the server's own JSON form, so a stored copy reads back
/// like a response.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
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
    /// OpenSubsonic servers only.
    release_date: Option<ReleaseDate>,
    created: Option<String>,
    played: Option<String>,
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

    /// The full date where the server has one, otherwise the year.
    pub fn release_date(&self) -> Option<ReleaseDate> {
        self.release_date
            .filter(ReleaseDate::has_year)
            .or_else(|| self.year.map(ReleaseDate::year))
    }

    /// When it was added to the library.
    pub fn created(&self) -> Option<SystemTime> {
        self.created.as_deref().and_then(parse_time)
    }

    /// When the user last played a song of it, in RFC 3339. `None` if they
    /// never have, or the server does not say.
    pub fn played(&self) -> Option<&str> {
        self.played.as_deref()
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn album(fields: serde_json::Value) -> Album {
        let mut album = json!({ "id": "a", "name": "Kind of Blue" });
        album
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        serde_json::from_value(album).unwrap()
    }

    #[test]
    fn the_release_date_falls_back_to_the_year() {
        let dated = album(json!({ "year": 1959, "releaseDate": { "year": 1959, "month": 8 } }));
        assert!(dated.release_date().unwrap() > ReleaseDate::year(1959));
        // OpenSubsonic servers send an empty date for an album without one
        let undated = album(json!({ "year": 1959, "releaseDate": {} }));
        assert_eq!(undated.release_date(), Some(ReleaseDate::year(1959)));
        assert_eq!(album(json!({})).release_date(), None);
    }

    #[test]
    fn a_stored_album_keeps_when_it_was_added() {
        let added = album(json!({ "created": "2024-05-01T12:00:00Z" }));
        let stored: Album = serde_json::from_value(serde_json::to_value(&added).unwrap()).unwrap();
        assert!(stored.created().is_some());
        assert_eq!(stored.created(), added.created());
    }
}
