//! OpenSubsonic client that runs [`subsonic`] endpoints over [`net`].
//!
//! Runtime-agnostic: its futures can be awaited from GPUI or the CLI.

use serde::de::DeserializeOwned;
use subsonic::{
    Album, AlbumId, AlbumWithSongs, CoverArtId, Credentials, Endpoint, Envelope, GetAlbum,
    GetAlbumList2, GetSong, Ping, Search3, ServerInfo, Song, SongId,
};

/// Largest page `getAlbumList2` returns.
const ALBUM_PAGE_SIZE: u32 = 500;
use url::Url;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("`{0}` is not a usable server address; use an http or https URL")]
    InvalidServerUrl(String),
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
        let invalid = || Error::InvalidServerUrl(base_url.to_owned());
        let parsed = Url::parse(base_url).map_err(|_| invalid())?;
        if !matches!(parsed.scheme(), "http" | "https") || parsed.cannot_be_a_base() {
            return Err(invalid());
        }
        Ok(Self {
            base_url: parsed,
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
            let page = self
                .call::<GetAlbumList2>(&Endpoint::albums_by_name(ALBUM_PAGE_SIZE, offset))
                .await?
                .into_payload()
                .into_albums();
            let last = page.len() < ALBUM_PAGE_SIZE as usize;
            albums.extend(page);
            if last {
                return Ok(albums);
            }
        }
    }

    pub async fn album(&self, album: &AlbumId) -> Result<AlbumWithSongs, Error> {
        let envelope = self.call::<GetAlbum>(&Endpoint::album(album)).await?;
        Ok(envelope.into_payload().into_album())
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
        assert!(Server::new("https://music.example.com", credentials).is_ok());
    }
}
