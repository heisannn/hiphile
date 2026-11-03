use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::{Album, AlbumWithSongs, Error, ServerErrorCode, Song};

/// Fields every `subsonic-response` carries.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    version: String,
    #[serde(rename = "type")]
    server_type: Option<String>,
    server_version: Option<String>,
    #[serde(default)]
    open_subsonic: bool,
}

impl ServerInfo {
    /// Subsonic protocol version the server implements.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Server software name, e.g. `navidrome`. OpenSubsonic servers only.
    pub fn server_type(&self) -> Option<&str> {
        self.server_type.as_deref()
    }

    /// Server software version. OpenSubsonic servers only.
    pub fn server_version(&self) -> Option<&str> {
        self.server_version.as_deref()
    }

    pub fn is_open_subsonic(&self) -> bool {
        self.open_subsonic
    }

    pub fn is_navidrome(&self) -> bool {
        self.server_type
            .as_deref()
            .is_some_and(|name| name.eq_ignore_ascii_case("navidrome"))
    }
}

/// A successful response: server info plus the method's payload.
#[derive(Debug)]
pub struct Envelope<T> {
    server: ServerInfo,
    payload: T,
}

impl<T> Envelope<T> {
    pub fn server(&self) -> &ServerInfo {
        &self.server
    }

    pub fn payload(&self) -> &T {
        &self.payload
    }

    pub fn into_payload(self) -> T {
        self.payload
    }
}

/// Payload of `ping`, which has no fields of its own.
#[derive(Debug, Deserialize)]
pub struct Ping {}

/// Payload of `getAlbumList2`.
#[derive(Debug, Deserialize)]
pub struct GetAlbumList2 {
    #[serde(rename = "albumList2")]
    list: AlbumList,
}

#[derive(Debug, Default, Deserialize)]
struct AlbumList {
    #[serde(default)]
    album: Vec<Album>,
}

impl GetAlbumList2 {
    pub fn into_albums(self) -> Vec<Album> {
        self.list.album
    }
}

/// Payload of `getIndexes`; only when the library last changed is read.
#[derive(Debug, Deserialize)]
pub struct GetIndexes {
    #[serde(default)]
    indexes: Indexes,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Indexes {
    last_modified: Option<u64>,
}

impl GetIndexes {
    /// When the library last changed, in milliseconds since the Unix epoch.
    /// Navidrome reports its last scan.
    pub fn last_modified(&self) -> Option<u64> {
        self.indexes.last_modified
    }
}

/// Payload of `getScanStatus`.
#[derive(Debug, Deserialize)]
pub struct GetScanStatus {
    #[serde(rename = "scanStatus")]
    status: ScanStatus,
}

impl GetScanStatus {
    pub fn into_status(self) -> ScanStatus {
        self.status
    }
}

/// Whether the server is scanning its media, how many items it counted, and
/// when its last scan ran.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanStatus {
    scanning: bool,
    count: Option<u64>,
    last_scan: Option<String>,
}

impl ScanStatus {
    pub fn new(scanning: bool, count: Option<u64>, last_scan: Option<String>) -> Self {
        Self {
            scanning,
            count,
            last_scan,
        }
    }

    pub fn is_scanning(&self) -> bool {
        self.scanning
    }

    pub fn count(&self) -> Option<u64> {
        self.count
    }

    /// When the last scan ran, as the server writes it. Navidrome only.
    pub fn last_scan(&self) -> Option<&str> {
        self.last_scan.as_deref()
    }
}

/// Payload of `getAlbum`.
#[derive(Debug, Deserialize)]
pub struct GetAlbum {
    album: AlbumWithSongs,
}

impl GetAlbum {
    pub fn into_album(self) -> AlbumWithSongs {
        self.album
    }
}

/// Payload of `getSong`.
#[derive(Debug, Deserialize)]
pub struct GetSong {
    song: Song,
}

impl GetSong {
    pub fn into_song(self) -> Song {
        self.song
    }
}

/// Payload of `search3`.
#[derive(Debug, Deserialize)]
pub struct Search3 {
    #[serde(rename = "searchResult3")]
    result: SearchResult3,
}

#[derive(Debug, Default, Deserialize)]
struct SearchResult3 {
    #[serde(default)]
    song: Vec<Song>,
}

impl Search3 {
    pub fn songs(&self) -> &[Song] {
        &self.result.song
    }

    pub fn into_songs(self) -> Vec<Song> {
        self.result.song
    }
}

#[derive(Deserialize)]
struct Root {
    #[serde(rename = "subsonic-response")]
    response: Body,
}

#[derive(Deserialize)]
struct Body {
    status: Status,
    #[serde(flatten)]
    server: ServerInfo,
    error: Option<ServerError>,
    /// Everything else; the method's payload is decoded from it only after the
    /// status says the call succeeded, so a failure is never reported as a
    /// payload mismatch.
    #[serde(flatten)]
    rest: serde_json::Map<String, serde_json::Value>,
}

#[derive(Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Status {
    Ok,
    Failed,
}

#[derive(Deserialize)]
struct ServerError {
    code: u32,
    #[serde(default)]
    message: String,
}

/// Parses a JSON (`f=json`) response body.
pub fn parse<T: DeserializeOwned>(body: &[u8]) -> Result<Envelope<T>, Error> {
    let body = serde_json::from_slice::<Root>(body)?.response;
    if body.status == Status::Failed {
        let error = body.error.unwrap_or(ServerError {
            code: ServerErrorCode::GENERIC.value(),
            message: String::new(),
        });
        return Err(Error::Server {
            code: ServerErrorCode::new(error.code),
            message: error.message,
        });
    }
    let payload = T::deserialize(serde_json::Value::Object(body.rest))?;
    Ok(Envelope {
        server: body.server,
        payload,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_navidrome_ping() {
        let body = br#"{"subsonic-response":{"status":"ok","version":"1.16.1","type":"navidrome","serverVersion":"0.53.3","openSubsonic":true}}"#;
        let envelope = parse::<Ping>(body).unwrap();
        assert!(envelope.server().is_navidrome());
        assert!(envelope.server().is_open_subsonic());
        assert_eq!(envelope.server().server_version(), Some("0.53.3"));
    }

    #[test]
    fn parses_legacy_subsonic_ping() {
        let body = br#"{"subsonic-response":{"status":"ok","version":"1.16.1"}}"#;
        let envelope = parse::<Ping>(body).unwrap();
        assert!(!envelope.server().is_open_subsonic());
        assert_eq!(envelope.server().server_type(), None);
    }

    #[test]
    fn maps_failed_status_to_server_error() {
        let body = br#"{"subsonic-response":{"status":"failed","version":"1.16.1","error":{"code":40,"message":"Wrong username or password"}}}"#;
        match parse::<Ping>(body) {
            Err(Error::Server { code, message }) => {
                assert!(code.is_authentication());
                assert_eq!(message, "Wrong username or password");
            }
            other => panic!("expected a server error, got {other:?}"),
        }
    }

    #[test]
    fn parses_search3_songs() {
        let body = br#"{"subsonic-response":{"status":"ok","version":"1.16.1","searchResult3":{"song":[{"id":"s1","isDir":false,"title":"Song","artist":"Artist","duration":215,"samplingRate":96000,"bitDepth":24,"channelCount":2,"suffix":"flac","size":31457280}]}}}"#;
        let songs = parse::<Search3>(body).unwrap().into_payload().into_songs();
        assert_eq!(songs.len(), 1);
        assert_eq!(songs[0].id().as_str(), "s1");
        assert_eq!(songs[0].sampling_rate(), Some(96000));
        assert_eq!(songs[0].size(), Some(31_457_280));
        assert_eq!(songs[0].duration().map(|d| d.as_secs()), Some(215));
    }

    #[test]
    fn empty_search_result_has_no_songs() {
        let body =
            br#"{"subsonic-response":{"status":"ok","version":"1.16.1","searchResult3":{}}}"#;
        assert!(parse::<Search3>(body).unwrap().payload().songs().is_empty());
    }

    #[test]
    fn failed_status_wins_over_missing_payload() {
        let body = br#"{"subsonic-response":{"status":"failed","version":"1.16.1","error":{"code":70,"message":"not found"}}}"#;
        assert!(matches!(
            parse::<Search3>(body),
            Err(Error::Server {
                code: ServerErrorCode::NOT_FOUND,
                ..
            })
        ));
    }

    #[test]
    fn payload_mismatch_is_an_invalid_response() {
        let body = br#"{"subsonic-response":{"status":"ok","version":"1.16.1"}}"#;
        assert!(matches!(
            parse::<Search3>(body),
            Err(Error::InvalidResponse(_))
        ));
    }

    #[test]
    fn parses_album_list_page() {
        let body = br#"{"subsonic-response":{"status":"ok","version":"1.16.1","albumList2":{"album":[{"id":"a1","name":"Kind of Blue","artist":"Miles Davis","coverArt":"al-a1","songCount":5,"duration":2756,"year":1959}]}}}"#;
        let albums = parse::<GetAlbumList2>(body)
            .unwrap()
            .into_payload()
            .into_albums();
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].name(), "Kind of Blue");
        assert_eq!(albums[0].cover_art().map(|id| id.as_str()), Some("al-a1"));
        assert_eq!(albums[0].year(), Some(1959));
    }

    #[test]
    fn empty_album_list_page_has_no_albums() {
        let body = br#"{"subsonic-response":{"status":"ok","version":"1.16.1","albumList2":{}}}"#;
        assert!(
            parse::<GetAlbumList2>(body)
                .unwrap()
                .into_payload()
                .into_albums()
                .is_empty()
        );
    }

    #[test]
    fn parses_album_with_songs() {
        let body = br#"{"subsonic-response":{"status":"ok","version":"1.16.1","album":{"id":"a1","name":"Kind of Blue","songCount":2,"song":[{"id":"s1","isDir":false,"title":"So What","track":1},{"id":"s2","isDir":false,"title":"Freddie Freeloader","track":2}]}}}"#;
        let album = parse::<GetAlbum>(body).unwrap().into_payload().into_album();
        assert_eq!(album.album().id().as_str(), "a1");
        assert_eq!(album.songs().len(), 2);
        assert_eq!(album.songs()[1].track(), Some(2));
    }

    #[test]
    fn parses_indexes_last_modified_without_artists() {
        let body = br#"{"subsonic-response":{"status":"ok","version":"1.16.1","indexes":{"lastModified":1730000000000,"ignoredArticles":"The"}}}"#;
        let indexes = parse::<GetIndexes>(body).unwrap().into_payload();
        assert_eq!(indexes.last_modified(), Some(1_730_000_000_000));
    }

    #[test]
    fn parses_scan_status() {
        let body = br#"{"subsonic-response":{"status":"ok","version":"1.16.1","scanStatus":{"scanning":false,"count":1234,"lastScan":"2026-11-01T10:00:00Z"}}}"#;
        let status = parse::<GetScanStatus>(body)
            .unwrap()
            .into_payload()
            .into_status();
        assert_eq!(
            status,
            ScanStatus::new(false, Some(1234), Some("2026-11-01T10:00:00Z".to_owned()))
        );
    }

    #[test]
    fn stored_album_and_song_read_back() {
        let album: Album = serde_json::from_str(
            r#"{"id":"a1","name":"Kind of Blue","artist":"Miles Davis","year":1959}"#,
        )
        .unwrap();
        let again: Album = serde_json::from_str(&serde_json::to_string(&album).unwrap()).unwrap();
        assert_eq!(again.name(), "Kind of Blue");
        assert_eq!(again.year(), Some(1959));

        let song: Song =
            serde_json::from_str(r#"{"id":"s1","isDir":false,"title":"So What","track":1}"#)
                .unwrap();
        let again: Song = serde_json::from_str(&serde_json::to_string(&song).unwrap()).unwrap();
        assert_eq!(again, song);
    }

    #[test]
    fn rejects_non_subsonic_body() {
        assert!(matches!(
            parse::<Ping>(b"<html></html>"),
            Err(Error::InvalidResponse(_))
        ));
    }
}
