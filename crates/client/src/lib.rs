//! OpenSubsonic client that runs [`subsonic`] endpoints over [`net`].
//!
//! Runtime-agnostic: its futures can be awaited from GPUI or the CLI.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use subsonic::{
    Album, AlbumId, AlbumWithSongs, Artist, ArtistId, ArtistWithAlbums, CoverArtId, Credentials,
    Endpoint, Envelope, GetAlbum, GetAlbumList2, GetArtist, GetArtistInfo2, GetArtists, GetIndexes,
    GetPlaylist, GetPlaylists, GetScanStatus, GetSimilarSongs2, GetSong, GetStarred2, GetTopSongs,
    Ping, Playlist, PlaylistId, PlaylistWithSongs, ScanStatus, Search3, ServerInfo, Song, SongId,
    StarTarget, Starred,
};
use url::Url;

/// Largest page `getAlbumList2` returns.
pub const ALBUM_PAGE_SIZE: u32 = 500;
/// How many songs [`Client::all_songs`] asks for at a time.
const SONG_PAGE_SIZE: u32 = 500;

/// Longest URL sent as is. Proxies in front of a server commonly refuse more
/// than 8 KiB of request line, and a playlist lists every one of its songs.
const LONGEST_URL: usize = 4096;

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
///
/// A response is decoded where its future is polled, so await calls off a UI
/// thread: a response can be large.
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

    /// The transport, for requests that are not to the server.
    pub fn net(&self) -> &net::Client {
        &self.net
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

    /// Every artist with an album, in the server's name order.
    pub async fn artists(&self) -> Result<Vec<Artist>, Error> {
        let envelope = self.call::<GetArtists>(&Endpoint::artists()).await?;
        Ok(envelope.into_payload().into_artists())
    }

    pub async fn artist(&self, artist: &ArtistId) -> Result<ArtistWithAlbums, Error> {
        let envelope = self.call::<GetArtist>(&Endpoint::artist(artist)).await?;
        Ok(envelope.into_payload().into_artist())
    }

    /// Up to `count` artists in the library similar to `artist`; none when the
    /// server has no outside service to ask.
    pub async fn similar_artists(
        &self,
        artist: &ArtistId,
        count: u32,
    ) -> Result<Vec<Artist>, Error> {
        let envelope = self
            .call::<GetArtistInfo2>(&Endpoint::artist_info(artist, count))
            .await?;
        Ok(envelope.into_payload().into_similar_artists())
    }

    /// The `count` most listened to songs of the artist called `artist`; none
    /// when the server has no outside service to ask.
    pub async fn top_songs(&self, artist: &str, count: u32) -> Result<Vec<Song>, Error> {
        let envelope = self
            .call::<GetTopSongs>(&Endpoint::top_songs(artist, count))
            .await?;
        Ok(envelope.into_payload().into_songs())
    }

    /// Every song in the library, fetched page by page.
    pub async fn all_songs(&self) -> Result<Vec<Song>, Error> {
        let mut songs = Vec::new();
        loop {
            let offset = u32::try_from(songs.len()).unwrap_or(u32::MAX);
            let page = self.songs_page(SONG_PAGE_SIZE, offset).await?;
            let last = page.len() < SONG_PAGE_SIZE as usize;
            songs.extend(page);
            if last {
                return Ok(songs);
            }
        }
    }

    /// Every playlist the user can play.
    pub async fn playlists(&self) -> Result<Vec<Playlist>, Error> {
        let envelope = self.call::<GetPlaylists>(&Endpoint::playlists()).await?;
        Ok(envelope.into_payload().into_playlists())
    }

    pub async fn playlist(&self, playlist: &PlaylistId) -> Result<PlaylistWithSongs, Error> {
        let envelope = self
            .call::<GetPlaylist>(&Endpoint::playlist(playlist))
            .await?;
        Ok(envelope.into_payload().into_playlist())
    }

    pub async fn create_playlist(
        &self,
        name: &str,
        songs: &[SongId],
    ) -> Result<PlaylistWithSongs, Error> {
        let envelope = self
            .call_with_form::<GetPlaylist>(&Endpoint::create_playlist(name, songs))
            .await?;
        Ok(envelope.into_payload().into_playlist())
    }

    /// Makes `songs`, in this order, the whole of `playlist`.
    pub async fn replace_playlist_songs(
        &self,
        playlist: &PlaylistId,
        songs: &[SongId],
    ) -> Result<(), Error> {
        if songs.is_empty() {
            return self.clear_playlist(playlist).await;
        }
        self.call_with_form::<Ping>(&Endpoint::replace_playlist_songs(playlist, songs))
            .await?;
        Ok(())
    }

    /// Takes every song out of `playlist` by position: `createPlaylist` with no
    /// songs may be read as no change to them.
    async fn clear_playlist(&self, playlist: &PlaylistId) -> Result<(), Error> {
        let count = self.playlist(playlist).await?.songs().len();
        if count > 0 {
            self.call_with_form::<Ping>(&Endpoint::remove_from_playlist(playlist, 0..count))
                .await?;
        }
        Ok(())
    }

    /// Adds `songs` after those already in `playlist`.
    pub async fn add_to_playlist(
        &self,
        playlist: &PlaylistId,
        songs: &[SongId],
    ) -> Result<(), Error> {
        self.call_with_form::<Ping>(&Endpoint::add_to_playlist(playlist, songs))
            .await?;
        Ok(())
    }

    pub async fn rename_playlist(&self, playlist: &PlaylistId, name: &str) -> Result<(), Error> {
        self.call::<Ping>(&Endpoint::rename_playlist(playlist, name))
            .await?;
        Ok(())
    }

    pub async fn delete_playlist(&self, playlist: &PlaylistId) -> Result<(), Error> {
        self.call::<Ping>(&Endpoint::delete_playlist(playlist))
            .await?;
        Ok(())
    }

    /// Every song, album and artist the user marked as a favorite.
    pub async fn starred(&self) -> Result<Starred, Error> {
        let envelope = self.call::<GetStarred2>(&Endpoint::starred()).await?;
        Ok(envelope.into_payload().into_starred())
    }

    /// Marks `target` as a favorite of the user.
    pub async fn star(&self, target: &StarTarget) -> Result<(), Error> {
        self.call::<Ping>(&Endpoint::star(target)).await?;
        Ok(())
    }

    /// Takes `target` out of the user's favorites.
    pub async fn unstar(&self, target: &StarTarget) -> Result<(), Error> {
        self.call::<Ping>(&Endpoint::unstar(target)).await?;
        Ok(())
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
        decode(response.status(), response.body())
    }

    /// Like [`Self::call`] for an endpoint whose parameters can outgrow a URL:
    /// those too long for one go in a form body, which OpenSubsonic servers
    /// read the same way (the `formPost` extension).
    async fn call_with_form<T: DeserializeOwned>(
        &self,
        endpoint: &Endpoint,
    ) -> Result<Envelope<T>, Error> {
        let (url, form) = request_parts(self.server.url(endpoint));
        let response = match form {
            Some(form) => self.net.post_form(url, form).await?,
            None => self.net.get(url).await?,
        };
        decode(response.status(), response.body())
    }
}

/// `url` as it is when short enough to send, or else without its query, which
/// is returned to send as a form.
fn request_parts(mut url: Url) -> (Url, Option<String>) {
    if url.as_str().len() <= LONGEST_URL {
        return (url, None);
    }
    let form = url.query().map(str::to_owned);
    url.set_query(None);
    (url, form)
}

fn decode<T: DeserializeOwned>(
    status: net::http::StatusCode,
    body: &[u8],
) -> Result<Envelope<T>, Error> {
    // Subsonic reports protocol errors with HTTP 200; only a body we can
    // parse should decide. A non-success status with no Subsonic body is a
    // proxy or routing problem.
    match subsonic::parse::<T>(body) {
        Err(subsonic::Error::InvalidResponse(_)) if !status.is_success() => {
            Err(Error::HttpStatus(status.as_u16()))
        }
        result => Ok(result?),
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
    fn a_url_too_long_to_send_moves_its_query_to_a_form() {
        let server = Server::new("https://music.example.com", Credentials::api_key("k")).unwrap();
        let playlist = PlaylistId::new("p1");
        let few = vec![SongId::new("song"); 3];
        let endpoint = Endpoint::replace_playlist_songs(&playlist, &few);
        let (url, form) = request_parts(server.url(&endpoint));
        assert_eq!(form, None);
        assert!(url.query().is_some());

        let many = vec![SongId::new("0123456789abcdef0123456789abcdef"); 500];
        let endpoint = Endpoint::replace_playlist_songs(&playlist, &many);
        let (url, form) = request_parts(server.url(&endpoint));
        assert_eq!(url.query(), None);
        assert_eq!(url.path(), "/rest/createPlaylist");
        let form = form.unwrap();
        assert_eq!(form.matches("songId=").count(), 500);
        assert!(form.contains("apiKey=k"));
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
