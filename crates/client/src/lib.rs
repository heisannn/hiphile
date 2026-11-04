//! OpenSubsonic client that runs [`subsonic`] endpoints over [`net`].
//!
//! Runtime-agnostic: its futures can be awaited from GPUI or the CLI.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use subsonic::{
    Album, AlbumId, AlbumWithSongs, CoverArtId, Credentials, Endpoint, Envelope, GetAlbum,
    GetAlbumList2, GetIndexes, GetScanStatus, GetSimilarSongs2, GetSong, Ping, ScanStatus, Search3,
    ServerInfo, Song, SongId,
};
use url::Url;

/// Largest page `getAlbumList2` returns.
pub const ALBUM_PAGE_SIZE: u32 = 500;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("`{0}` is not a usable server address; use an http or https URL")]
    InvalidServerUrl(String),
    #[error(
        "`{0}` is not a usable server address; leave out any user name, password, `?` or `#` part"
    )]
    ServerUrlHasExtras(String),
    #[error("could not reach the server")]
    Network(#[source] net::Error),
    #[error("the server answered with HTTP {0}")]
    HttpStatus(u16),
    #[error(transparent)]
    Protocol(#[from] subsonic::Error),
    #[error("the server sent `{0}` instead of audio")]
    NotAudio(String),
}

impl From<net::Error> for Error {
    fn from(error: net::Error) -> Self {
        match error {
            net::Error::HttpStatus(status) => Self::HttpStatus(status),
            error => Self::Network(error),
        }
    }
}

/// Where the server lives and how to authenticate with it.
#[derive(Clone, Debug)]
pub struct Server {
    base_url: Url,
    credentials: Credentials,
}

impl Server {
    pub fn new(base_url: &str, credentials: Credentials) -> Result<Self, Error> {
        Ok(Self {
            base_url: server_url(base_url)?,
            credentials,
        })
    }

    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    pub fn url(&self, endpoint: &Endpoint) -> Url {
        endpoint.url(&self.base_url, &self.credentials)
    }
}

/// Parses a server address the way [`Server::new`] does, for uses that need
/// no credentials, such as finding the server's cached library.
pub fn server_url(address: &str) -> Result<Url, Error> {
    let invalid = || Error::InvalidServerUrl(address.to_owned());
    let parsed = Url::parse(address).map_err(|_| invalid())?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.cannot_be_a_base() {
        return Err(invalid());
    }
    // Requests add their own query to the address, and credentials go in
    // them separately; anything there would be sent along or lost.
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(Error::ServerUrlHasExtras(address.to_owned()));
    }
    Ok(parsed)
}

/// Cheap to clone; clones share the transport.
#[derive(Clone)]
pub struct Client {
    net: net::Client,
    server: Server,
}

impl Client {
    pub fn new(net: net::Client, server: Server) -> Self {
        Self { net, server }
    }

    pub fn server(&self) -> &Server {
        &self.server
    }

    pub async fn ping(&self) -> Result<ServerInfo, Error> {
        let envelope = self.call::<Ping>(&Endpoint::ping()).await?;
        Ok(envelope.server().clone())
    }

    /// Every album, sorted by name, fetched page by page.
    pub async fn albums_by_name(&self) -> Result<Vec<Album>, Error> {
        let mut albums = Vec::new();
        loop {
            let offset = u32::try_from(albums.len()).unwrap_or(u32::MAX);
            let page = self.albums_page(ALBUM_PAGE_SIZE, offset).await?;
            let last = page.len() < ALBUM_PAGE_SIZE as usize;
            albums.extend(page);
            if last {
                return Ok(albums);
            }
        }
    }

    /// One page of the albums sorted by name; `size` is at most
    /// [`ALBUM_PAGE_SIZE`].
    pub async fn albums_page(&self, size: u32, offset: u32) -> Result<Vec<Album>, Error> {
        let envelope = self
            .call::<GetAlbumList2>(&Endpoint::albums_by_name(size, offset))
            .await?;
        Ok(envelope.into_payload().into_albums())
    }

    /// The `size` albums added to the library last, newest first.
    pub async fn newest_albums(&self, size: u32) -> Result<Vec<Album>, Error> {
        let envelope = self
            .call::<GetAlbumList2>(&Endpoint::newest_albums(size))
            .await?;
        Ok(envelope.into_payload().into_albums())
    }

    /// The `size` albums played last by the user, most recent first.
    pub async fn recently_played_albums(&self, size: u32) -> Result<Vec<Album>, Error> {
        let envelope = self
            .call::<GetAlbumList2>(&Endpoint::recently_played_albums(size))
            .await?;
        Ok(envelope.into_payload().into_albums())
    }

    /// The `size` albums the user played most, most played first.
    pub async fn frequent_albums(&self, size: u32) -> Result<Vec<Album>, Error> {
        let envelope = self
            .call::<GetAlbumList2>(&Endpoint::frequent_albums(size))
            .await?;
        Ok(envelope.into_payload().into_albums())
    }

    /// `size` albums in no order, different each time.
    pub async fn random_albums(&self, size: u32) -> Result<Vec<Album>, Error> {
        let envelope = self
            .call::<GetAlbumList2>(&Endpoint::random_albums(size))
            .await?;
        Ok(envelope.into_payload().into_albums())
    }

    /// Up to `count` songs similar to those of `album`; none when the server
    /// has no outside service to ask.
    pub async fn similar_songs(&self, album: &AlbumId, count: u32) -> Result<Vec<Song>, Error> {
        let envelope = self
            .call::<GetSimilarSongs2>(&Endpoint::similar_songs(album, count))
            .await?;
        Ok(envelope.into_payload().into_songs())
    }

    /// One page of every song in the library, in an order that stays the
    /// same between pages while the library does not change.
    pub async fn songs_page(&self, size: u32, offset: u32) -> Result<Vec<Song>, Error> {
        let envelope = self
            .call::<Search3>(&Endpoint::all_songs(size, offset))
            .await?;
        Ok(envelope.into_payload().into_songs())
    }

    pub async fn scan_status(&self) -> Result<ScanStatus, Error> {
        let envelope = self.call::<GetScanStatus>(&Endpoint::scan_status()).await?;
        Ok(envelope.into_payload().into_status())
    }

    /// When the library last changed, in milliseconds since the Unix epoch,
    /// if the server says. Navidrome reports its last scan.
    pub async fn library_modified(&self) -> Result<Option<u64>, Error> {
        // The index of every artist is sent only if it changed after the
        // time asked about; asking about now leaves just the time it changed.
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| {
                u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
            });
        let envelope = self
            .call::<GetIndexes>(&Endpoint::indexes_modified_since(now))
            .await?;
        Ok(envelope.into_payload().last_modified())
    }

    pub async fn album(&self, album: &AlbumId) -> Result<AlbumWithSongs, Error> {
        let envelope = self.call::<GetAlbum>(&Endpoint::album(album)).await?;
        Ok(envelope.into_payload().into_album())
    }

    /// Tells the server `song` was listened to at `played_at`.
    pub async fn scrobble(&self, song: &SongId, played_at: SystemTime) -> Result<(), Error> {
        let played_at = played_at.duration_since(UNIX_EPOCH).map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        });
        self.call::<Ping>(&Endpoint::scrobble(song, played_at))
            .await?;
        Ok(())
    }

    /// Tells the server `song` is playing now, without counting it as played.
    pub async fn now_playing(&self, song: &SongId) -> Result<(), Error> {
        self.call::<Ping>(&Endpoint::now_playing(song)).await?;
        Ok(())
    }

    /// URL of a cover art image, authenticated like any other request.
    ///
    /// Each call yields a different URL (a fresh salt), so keep the result
    /// for as long as the image is shown; an image cache keyed by URL would
    /// otherwise fetch it again on every call. The URL carries credentials:
    /// never log or display it.
    pub fn cover_art_url(&self, cover_art: &CoverArtId, size: Option<u32>) -> Url {
        self.server.url(&Endpoint::cover_art(cover_art, size))
    }

    pub async fn song(&self, song: &SongId) -> Result<Song, Error> {
        let envelope = self.call::<GetSong>(&Endpoint::song(song)).await?;
        Ok(envelope.into_payload().into_song())
    }

    pub async fn search_songs(&self, query: &str, count: u32) -> Result<Vec<Song>, Error> {
        let envelope = self
            .call::<Search3>(&Endpoint::search_songs(query, count))
            .await?;
        Ok(envelope.into_payload().into_songs())
    }

    /// Opens the original file of `song` as a download that can be decoded
    /// while it arrives.
    pub async fn open_stream(&self, song: &SongId) -> Result<net::Download, Error> {
        let url = self.server.url(&Endpoint::stream_original(song));
        let download = self.net.download(url).await?;
        // `stream` reports failures as a Subsonic document with HTTP 200.
        let content_type = download.content_type().unwrap_or_default().to_owned();
        if is_document(&content_type) {
            let body = download.into_body().await?;
            subsonic::parse::<Ping>(&body)?;
            return Err(Error::NotAudio(content_type));
        }
        Ok(download)
    }

    pub async fn call<T: DeserializeOwned>(
        &self,
        endpoint: &Endpoint,
    ) -> Result<Envelope<T>, Error> {
        let response = self.net.get(self.server.url(endpoint)).await?;
        let status = response.status();
        // Subsonic reports protocol errors with HTTP 200; only a body we can
        // parse should decide. A non-success status with no Subsonic body is a
        // proxy or routing problem.
        match subsonic::parse::<T>(response.body()) {
            Err(subsonic::Error::InvalidResponse(_)) if !status.is_success() => {
                Err(Error::HttpStatus(status.as_u16()))
            }
            result => Ok(result?),
        }
    }
}

fn is_document(content_type: &str) -> bool {
    let essence = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    matches!(
        essence.as_str(),
        "application/json" | "text/json" | "application/xml" | "text/xml" | "text/html"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_document_content_types() {
        assert!(is_document("application/json; charset=utf-8"));
        assert!(is_document("text/xml"));
        assert!(!is_document("audio/flac"));
        assert!(!is_document(""));
    }

    #[test]
    fn rejects_non_http_server_urls() {
        let credentials = Credentials::api_key("k");
        assert!(Server::new("music.example.com", credentials.clone()).is_err());
        assert!(Server::new("ftp://music.example.com", credentials.clone()).is_err());
        assert!(Server::new("https://example.com/navidrome/", credentials.clone()).is_ok());
        for address in [
            "https://alice:secret@music.example.com",
            "https://music.example.com/?id=a",
            "https://music.example.com/#top",
        ] {
            assert!(matches!(
                Server::new(address, credentials.clone()),
                Err(Error::ServerUrlHasExtras(_))
            ));
        }
        assert!(Server::new("https://music.example.com", credentials).is_ok());
    }
}
